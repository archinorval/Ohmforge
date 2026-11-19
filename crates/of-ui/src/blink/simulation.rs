use std::error::Error;

use of_analog::devices::PinDriver;
use of_analog::{AnalogEngine, EngineConfig, Options};
use of_components::analog::{Board, Library, Placement, build};
use of_core::{EngineId, Kernel, SimTime, Stop};
use of_firmware::ElfImage;
use of_parts::parse_part;
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, all_pin_codes};

pub(super) const END: SimTime = SimTime::from_us(2_600);
pub(super) const LED_ON_MA: f64 = 1.0;
const FIRMWARE: &[u8] = include_bytes!("../../../../firmware/esp32c3-blinky/blinky.elf");
const RESISTOR: &str = include_str!("../../../../parts/passives/resistor/part.toml");
const LED: &str = include_str!("../../../../parts/leds/red-0603/part.toml");
const VDD: f64 = 3.3;
pub(super) const R_SERIES: f64 = 330.0;
pub(super) const GPIO: u8 = 8;
pub(super) const RESISTOR_REF: &str = "R1";
pub(super) const LED_REF: &str = "D1";
const DRIVER: &str = "u1_gpio8";

pub(super) struct Blink {
    kernel: Kernel,
    cpu: EngineId,
    analog: EngineId,
}

impl Blink {
    pub(super) fn new() -> Result<Self, Box<dyn Error>> {
        let mut lib = Library::new();
        lib.add(parse_part(RESISTOR)?, None)?;
        lib.add(parse_part(LED)?, None)?;
        let board = Board {
            placements: vec![
                Placement::new(RESISTOR_REF, "generic.resistor", &[("1", "gpio8"), ("2", "led_a")])
                    .with("resistance", R_SERIES),
                Placement::new(LED_REF, "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
            ],
            ..Board::default()
        };
        let mut circuit = build(&lib, &board)?.circuit;
        let pin = circuit.node("gpio8");
        circuit.add(PinDriver::new(DRIVER, pin, 0, 0.0, 1e9))?;
        let mut analog = AnalogEngine::new(
            "analog",
            circuit,
            Options::default(),
            EngineConfig {
                max_step: 5e-6,
                record: true,
                ..EngineConfig::default()
            },
        )?;
        for (code, out) in all_pin_codes(GPIO) {
            let (voltage, resistance) = out.thevenin(VDD);
            analog.on_code(
                code,
                vec![
                    of_analog::SetParam {
                        device: DRIVER.into(),
                        param: "v".into(),
                        value: voltage,
                    },
                    of_analog::SetParam {
                        device: DRIVER.into(),
                        param: "r".into(),
                        value: resistance,
                    },
                ],
            );
        }

        let mut soc = Esp32c3::new();
        soc.load_elf(&ElfImage::parse(FIRMWARE)?)?;
        // Register the CPU first so its GPIO changes reach the analog solver in time.
        let mut kernel = Kernel::new(SimTime::from_us(50));
        let cpu = kernel.add_engine(Box::new(Esp32c3Engine::new("esp32c3", soc)));
        let analog = kernel.add_engine(Box::new(analog));
        kernel
            .engine_as::<Esp32c3Engine>(cpu)
            .expect("registered ESP32-C3")
            .link_pin(PinLink {
                pin: GPIO,
                target: analog.0,
            });
        Ok(Self { kernel, cpu, analog })
    }

    pub(super) fn advance(&mut self, seconds: f64) -> Result<(), Box<dyn Error>> {
        let until = SimTime::from_secs_f64(seconds).min(END);
        if until > self.kernel.now() {
            let stop = self.kernel.run_until(until)?;
            if !matches!(stop, Stop::Reached(_)) {
                return Err(format!("ESP32-C3 stopped at {:.3} ms ({stop:?})", self.time() * 1e3).into());
            }
        }
        Ok(())
    }

    pub(super) fn finished(&self) -> bool {
        self.kernel.now() == END
    }

    pub(super) fn time(&self) -> f64 {
        self.kernel.now().as_secs_f64()
    }

    pub(super) fn uart(&mut self) -> String {
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .uart_text()
    }

    fn analog(&mut self) -> &AnalogEngine {
        self.kernel
            .engine_as::<AnalogEngine>(self.analog)
            .expect("registered analog solver")
    }

    pub(super) fn current_ma(&mut self) -> f64 {
        let analog = self.analog();
        let gpio = analog.value("v(gpio8)").expect("GPIO8 voltage");
        let anode = analog.value("v(led_a)").expect("LED anode voltage");
        current_ma(gpio, anode)
    }

    pub(super) fn trace(&mut self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let trace = self.analog().waveforms().expect("waveform recording enabled");
        let gpio = trace.signal("v(gpio8)").expect("recorded GPIO8 voltage");
        let anode = trace.signal("v(led_a)").expect("recorded LED anode voltage");
        trace
            .x
            .iter()
            .zip(gpio)
            .zip(anode)
            .map(|((&t, &gpio), &anode)| (t, current_ma(gpio, anode)))
    }
}

fn current_ma(gpio: f64, anode: f64) -> f64 {
    (gpio - anode) * 1e3 / R_SERIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_blink_advances_firmware_and_records_led_current() {
        let mut sim = Blink::new().unwrap();
        assert_eq!(sim.time(), 0.0);
        assert!(sim.uart().is_empty());

        sim.advance(125e-6).unwrap();
        assert!((3.0..4.5).contains(&sim.current_ma()));
        assert_eq!(sim.uart(), "ohmforge blinky\n");

        sim.advance(375e-6).unwrap();
        assert!(sim.current_ma().abs() < 1e-3);
        sim.advance(625e-6).unwrap();
        assert!((3.0..4.5).contains(&sim.current_ma()));

        sim.advance(1.0).unwrap();
        assert!(sim.finished());
        let trace: Vec<_> = sim.trace().collect();
        assert!(trace.iter().any(|&(_, ma)| ma > LED_ON_MA));
        assert!(trace.iter().any(|&(_, ma)| ma.abs() < 1e-3));
        assert!((trace.last().unwrap().0 - END.as_secs_f64()).abs() < 1e-12);
    }
}
