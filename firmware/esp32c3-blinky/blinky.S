/* SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Ohmforge test firmware: ESP32-C3 blinky, bare metal, no ROM calls.
 *
 * Prints a banner on UART0, routes GPIO8 through the IO MUX to the GPIO
 * matrix, then toggles it every HALF_PERIOD SYSTIMER ticks (16 MHz), so the
 * full period is 2 * HALF_PERIOD / 16 MHz = 500 us. Each rising edge
 * increments `toggles` (.bss). Deadlines are absolute, so the period does
 * not drift with loop overhead.
 */

    .equ UART0,       0x60000000
    .equ GPIO,        0x60004000
    .equ IO_MUX,      0x60009000
    .equ SYSTIMER,    0x60023000
    .equ PIN,         8
    .equ HALF_PERIOD, 4000

    .section .text.entry, "ax"
    .globl _start
_start:
    li   sp, 0x3fcdfff0

    /* Banner. */
    la   a0, banner
    li   a1, UART0
1:  lbu  t0, 0(a0)
    beqz t0, 2f
    sw   t0, 0(a1)
    addi a0, a0, 1
    j    1b
2:
    /* IO MUX: function 1 (GPIO matrix), drive strength 2. */
    li   t1, IO_MUX + 4 + 4 * PIN
    li   t2, (1 << 12) | (2 << 10)
    sw   t2, 0(t1)
    /* GPIO matrix: OUT_SEL 128 = GPIO_OUT bit PIN. */
    li   s0, GPIO
    li   t1, 128
    sw   t1, 0x554 + 4 * PIN(s0)
    li   s2, 1 << PIN
    sw   s2, 0x24(s0)           /* GPIO_ENABLE_W1TS */

    call systimer_now
    mv   s1, a0                 /* next deadline */
    li   s3, HALF_PERIOD
    la   s4, toggles

loop:
    sw   s2, 0x08(s0)           /* GPIO_OUT_W1TS: LED on */
    lw   t0, 0(s4)
    addi t0, t0, 1
    sw   t0, 0(s4)
    add  s1, s1, s3
    call wait_until
    sw   s2, 0x0c(s0)           /* GPIO_OUT_W1TC: LED off */
    add  s1, s1, s3
    call wait_until
    j    loop

/* a0 = SYSTIMER unit 0 value, low 32 bits. Clobbers t5, t6. */
    .globl systimer_now
systimer_now:
    li   t5, SYSTIMER
    li   t6, 1 << 30
    sw   t6, 4(t5)              /* UNIT0_OP.UPDATE */
3:  lw   a0, 4(t5)
    slli a0, a0, 2              /* VALUE_VALID (bit 29) -> sign bit */
    bgez a0, 3b
    lw   a0, 0x44(t5)           /* UNIT0_VALUE_LO */
    ret

/* Spin until SYSTIMER reaches s1 (wrap-safe). */
    .globl wait_until
wait_until:
    mv   s5, ra
4:  call systimer_now
    sub  a0, a0, s1
    bltz a0, 4b
    mv   ra, s5
    ret

    .section .rodata
banner:
    .asciz "ohmforge blinky\n"

    .section .data
    .globl magic
magic:
    .word 0x0f0f1234

    .section .bss
    .globl toggles
toggles:
    .word 0
