/* SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Ohmforge test firmware: ESP32-C3 timer interrupts, bare metal, no ROM
 * calls.
 *
 * Prints a banner on UART0, switches the watchdogs off the way esp-hal
 * does (RWDT, both MWDTs, super watchdog on auto-feed), then:
 *
 * - SYSTIMER comparator 0 in period mode (HALF_PERIOD ticks of 16 MHz =
 *   250 us) -> source 37 -> CPU interrupt 1, priority 1: the handler
 *   toggles GPIO8 and counts `sys_ticks`;
 * - TIMG0 timer 0 at 1 MHz (APB / 80) with a 1000-tick auto-reload alarm
 *   (1 ms) -> source 32 -> CPU interrupt 2, priority 2: the handler re-arms
 *   the alarm and counts `tg_ticks`.
 *
 * The main loop only sleeps in `wfi`. Interrupts use vectored `mtvec`; an
 * unexpected trap stores `mcause` in `fault` and spins.
 */

    .equ UART0,       0x60000000
    .equ GPIO,        0x60004000
    .equ RTC_CNTL,    0x60008000
    .equ IO_MUX,      0x60009000
    .equ TIMG0,       0x6001f000
    .equ TIMG1,       0x60020000
    .equ SYSTIMER,    0x60023000
    .equ INTMATRIX,   0x600c2000
    .equ WDT_WKEY,    0x50d83aa1
    .equ SWD_WKEY,    0x8f1d312a
    .equ PIN,         8
    .equ HALF_PERIOD, 4000

    .section .text.entry, "ax"
    .globl _start
_start:
    li   sp, 0x3fcdfff0
    la   t0, vectors
    ori  t0, t0, 1              /* vectored mode */
    csrw mtvec, t0

    /* Banner. */
    la   a0, banner
    li   a1, UART0
1:  lbu  t0, 0(a0)
    beqz t0, 2f
    sw   t0, 0(a1)
    addi a0, a0, 1
    j    1b
2:
    /* Watchdogs off. RWDT: unlock, clear EN and FLASHBOOT_MOD_EN, lock. */
    li   t0, RTC_CNTL
    li   t1, WDT_WKEY
    sw   t1, 0xa8(t0)           /* RTC_CNTL_WDTWPROTECT */
    sw   zero, 0x90(t0)         /* RTC_CNTL_WDTCONFIG0 */
    sw   zero, 0xa8(t0)
    /* Super watchdog: unlock, auto-feed, lock. */
    li   t1, SWD_WKEY
    sw   t1, 0xb0(t0)           /* RTC_CNTL_SWD_WPROTECT */
    lw   t2, 0xac(t0)           /* RTC_CNTL_SWD_CONF */
    li   t3, 1 << 31            /* SWD_AUTO_FEED_EN */
    or   t2, t2, t3
    sw   t2, 0xac(t0)
    sw   zero, 0xb0(t0)
    /* MWDT0 and MWDT1: unlock, EN = FLASHBOOT_MOD_EN = 0 with
     * WDT_CONF_UPDATE_EN, lock. */
    li   t1, WDT_WKEY
    li   t2, 1 << 22
    li   t0, TIMG0
    sw   t1, 0x64(t0)           /* TIMG_WDTWPROTECT */
    sw   t2, 0x48(t0)           /* TIMG_WDTCONFIG0 */
    sw   zero, 0x64(t0)
    li   t0, TIMG1
    sw   t1, 0x64(t0)
    sw   t2, 0x48(t0)
    sw   zero, 0x64(t0)

    /* GPIO8 as a push-pull output through the GPIO matrix (as blinky). */
    li   t1, IO_MUX + 4 + 4 * PIN
    li   t2, (1 << 12) | (2 << 10)
    sw   t2, 0(t1)
    li   s0, GPIO
    li   t1, 128
    sw   t1, 0x554 + 4 * PIN(s0)
    li   t1, 1 << PIN
    sw   t1, 0x24(s0)           /* GPIO_ENABLE_W1TS */

    /* Interrupt matrix: SYSTIMER_TARGET0 -> 1, TG0_T0 -> 2. */
    li   t0, INTMATRIX
    li   t1, 1
    sw   t1, 4 * 37(t0)         /* SYSTIMER_TARGET0_INT_MAP */
    sw   t1, 0x114 + 4 * 1(t0)  /* CPU_INT_PRI_1 = 1 */
    sw   t1, 0x194(t0)          /* CPU_INT_THRESH = 1 */
    li   t1, 2
    sw   t1, 4 * 32(t0)         /* TG0_T0_INT_MAP */
    sw   t1, 0x114 + 4 * 2(t0)  /* CPU_INT_PRI_2 = 2 */
    li   t1, (1 << 1) | (1 << 2)
    sw   t1, 0x104(t0)          /* CPU_INT_ENABLE */

    /* SYSTIMER comparator 0: period mode on unit 0. */
    li   t0, SYSTIMER
    li   t1, (1 << 30) | HALF_PERIOD
    sw   t1, 0x34(t0)           /* TARGET0_CONF: PERIOD_MODE | PERIOD */
    li   t1, 1
    sw   t1, 0x50(t0)           /* COMP0_LOAD */
    sw   t1, 0x64(t0)           /* INT_ENA: target 0 */
    lw   t1, 0(t0)
    li   t2, 1 << 24
    or   t1, t1, t2
    sw   t1, 0(t0)              /* CONF.TARGET0_WORK_EN */

    /* TIMG0 timer 0: 1 MHz, alarm at 1000, auto-reload from 0. */
    li   t0, TIMG0
    li   t1, 1000
    sw   t1, 0x10(t0)           /* T0ALARMLO */
    sw   zero, 0x14(t0)         /* T0ALARMHI */
    sw   zero, 0x18(t0)         /* T0LOADLO */
    sw   zero, 0x1c(t0)         /* T0LOADHI */
    sw   zero, 0x20(t0)         /* T0LOAD */
    li   t1, 1
    sw   t1, 0x70(t0)           /* INT_ENA_TIMERS: T0 */
    li   t1, (1 << 31) | (1 << 30) | (1 << 29) | (80 << 13) | (1 << 10)
    sw   t1, 0(t0)              /* T0CONFIG: EN INCREASE AUTORELOAD DIV=80 ALARM_EN */

    csrsi mstatus, 8            /* MIE */
idle:
    wfi
    j    idle

/* CPU interrupt 1: SYSTIMER comparator 0. */
systimer_isr:
    addi sp, sp, -16
    sw   t0, 0(sp)
    sw   t1, 4(sp)
    li   t0, SYSTIMER
    li   t1, 1
    sw   t1, 0x6c(t0)           /* INT_CLR: target 0 */
    li   t0, GPIO
    lw   t1, 0x04(t0)           /* GPIO_OUT */
    xori t1, t1, 1 << PIN
    sw   t1, 0x04(t0)
    la   t0, sys_ticks
    lw   t1, 0(t0)
    addi t1, t1, 1
    sw   t1, 0(t0)
    lw   t0, 0(sp)
    lw   t1, 4(sp)
    addi sp, sp, 16
    mret

/* CPU interrupt 2: TIMG0 timer 0 alarm. */
timg0_isr:
    addi sp, sp, -16
    sw   t0, 0(sp)
    sw   t1, 4(sp)
    li   t0, TIMG0
    li   t1, 1
    sw   t1, 0x7c(t0)           /* INT_CLR_TIMERS: T0 */
    lw   t1, 0(t0)
    ori  t1, t1, 1 << 10        /* ALARM_EN again (hardware cleared it) */
    sw   t1, 0(t0)
    la   t0, tg_ticks
    lw   t1, 0(t0)
    addi t1, t1, 1
    sw   t1, 0(t0)
    lw   t0, 0(sp)
    lw   t1, 4(sp)
    addi sp, sp, 16
    mret

unexpected:
    csrr t0, mcause
    la   t1, fault
    sw   t0, 0(t1)
3:  j    3b

/* Vector table: entry n handles mcause n (entry 0 takes exceptions). The
 * C3 needs mtvec 256-byte aligned; entries must be 4-byte jumps. */
    .balign 256
    .option push
    .option norvc
vectors:
    j    unexpected
    j    systimer_isr
    j    timg0_isr
    .rept 29
    j    unexpected
    .endr
    .option pop

    .section .rodata
banner:
    .asciz "ohmforge timer irq\n"

    .section .bss
    .globl sys_ticks, tg_ticks, fault
sys_ticks:
    .word 0
tg_ticks:
    .word 0
fault:
    .word 0
