//! A parts-built board under the of-core kernel with a stand-in CPU: the
//! regulator's power-good crossing "boots" the CPU, which then drives a
//! GPIO into the LED switch. This is the P2 co-simulation path with the
//! CPU replaced by a script.

mod common;

use of_analog::devices::PinDriver;
use of_analog::netlist::{BuildEnv, add_deck};
use of_analog::{AnalogEngine, Edge, EngineConfig, Options, SetParam};
use of_components::analog::{Board, Placement, build};
use of_core::{Engine, EngineError, Event, Kernel, SimTime, Stop, SyncCtx};

const POWER_GOOD: u32 = 1;
const GPIO_HIGH: u32 = 2;

/// Boots on power-good, then sets the GPIO 500 µs later.
struct ScriptCpu {
    t: SimTime,
    analog: u32,
    boot: Option<SimTime>,
    sent: bool,
}

impl Engine for ScriptCpu {
    fn name(&self) -> &str {
        "cpu"
    }
    fn local_time(&self) -> SimTime {
        self.t
    }
    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        if let Some(boot) = self.boot
            && !self.sent
        {
            let at = boot.saturating_add(SimTime::from_us(500));
            if at <= limit {
                self.sent = true;
                cx.schedule(at, self.analog, GPIO_HIGH);
                self.t = at;
                return Ok(at);
            }
        }
        self.t = limit;
        Ok(limit)
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        if ev.code == POWER_GOOD {
            self.boot.get_or_insert(ev.at);
        }
        Ok(())
    }
}

#[test]
fn power_good_boots_the_cpu_which_lights_the_led() {
    let lib = common::library();
    let board = Board {
        placements: vec![
            Placement::new("CIN", "generic.capacitor-mlcc", &[("1", "usb"), ("2", "0")]).with("capacitance", 10e-6),
            Placement::new(
                "U1",
                "ams.ams1117-3.3",
                &[("VIN", "usb"), ("VOUT", "vdd"), ("GND", "0")],
            ),
            Placement::new("COUT", "generic.capacitor-tantalum", &[("+", "vdd"), ("-", "0")]),
            Placement::new("RB", "generic.resistor", &[("1", "gp"), ("2", "b")]).with("resistance", 1e3),
            Placement::new("Q1", "generic.npn-small-signal", &[("C", "sw"), ("B", "b"), ("E", "0")]),
            Placement::new("R2", "generic.resistor", &[("1", "vdd"), ("2", "led2a")]).with("resistance", 150.0),
            Placement::new("D2", "generic.led-red-0603", &[("A", "led2a"), ("K", "sw")]),
        ],
        ..Board::default()
    };
    let mut b = build(&lib, &board).unwrap();
    let stim = of_netlist::spice::parse_str("usb\nVUSB usb0 0 PWL(0 0 1m 5)\nRUSB usb0 usb 0.2\n.end\n").unwrap();
    add_deck(&mut b.circuit, &stim, &BuildEnv::default(), &mut Vec::new()).unwrap();
    let gp = b.circuit.node("gp");
    b.circuit.add(PinDriver::new("gpio", gp, 0, 0.0, 25.0)).unwrap();

    let cfg = EngineConfig {
        max_step: 5e-6,
        ..EngineConfig::default()
    };
    let mut eng = AnalogEngine::new("analog", b.circuit, Options::default(), cfg).unwrap();
    eng.on_code(
        GPIO_HIGH,
        vec![SetParam {
            device: "gpio".into(),
            param: "v".into(),
            value: 3.3,
        }],
    );
    let mut k = Kernel::new(SimTime::from_us(20));
    let cpu = k.add_engine(Box::new(ScriptCpu {
        t: SimTime::ZERO,
        analog: 1,
        boot: None,
        sent: false,
    }));
    eng.watch("v(vdd)", 3.0, Edge::Rising, cpu.0, POWER_GOOD).unwrap();
    let analog = k.add_engine(Box::new(eng));

    // Before the GPIO goes high the LED is dark.
    k.run_until(SimTime::from_ms(1)).unwrap();
    let boot = k.engine_as::<ScriptCpu>(cpu).unwrap().boot.expect("power-good fired");
    // vdd follows the USB ramp minus about 1.1 V of dropout until regulation:
    // 3.0 V is reached near 4.1 V in, i.e. about 0.8 ms into the 1 ms ramp.
    let tb = boot.as_secs_f64();
    assert!((0.7e-3..0.9e-3).contains(&tb), "boot at {tb}");
    let eng = k.engine_as::<AnalogEngine>(analog).unwrap();
    let dark = (eng.value("v(vdd)").unwrap() - eng.value("v(led2a)").unwrap()) / 150.0;
    assert!(dark.abs() < 1e-6, "LED current before the GPIO write {dark}");

    assert_eq!(
        k.run_until(SimTime::from_ms(3)).unwrap(),
        Stop::Reached(SimTime::from_ms(3))
    );
    let eng = k.engine_as::<AnalogEngine>(analog).unwrap();
    let vdd = eng.value("v(vdd)").unwrap();
    let led = (vdd - eng.value("v(led2a)").unwrap()) / 150.0;
    assert!((3.28..3.32).contains(&vdd), "vdd {vdd}");
    assert!((6e-3..12e-3).contains(&led), "LED current {led}");
    assert!(eng.value("v(sw)").unwrap() < 0.3, "Q1 saturated");
}
