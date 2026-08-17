# Raspberry Pi Pico [WIP]

The `rpi-pico` capture plugin allows developers to use any popular RP2040 or RP2350-based development board as a logic analyzer or
oscilloscope with OpenMSO.

It is currently a **work in progress**.

In OpenMSO, capturing from a microcontroller development board is valued primarily because it lowers the barrier to entry
for beginner of occasional hardware hackers who do not own dedicated test equipment.

## Capture plugin scope

The first goal is to support the 4 official Pico boards.

| Board                | Status         |
|----------------------|----------------|
| Raspberry Pi Pico    | In Development |
| Raspberry Pi Pico W  | Planned        |
| Raspberry Pi Pico 2  | Planned        |
| Raspberry Pi Pico 2W | Planned        |

Once these work, this plugin may be extended to include popular boards with the same microcontrollers - eg. [this list](https://en.wikipedia.org/wiki/RP2040#Boards).

Out of scope is:

- Anything which breaks the model of shipping exactly one `uf2` file, owned by this project, for each supported board.
- Support for hardware which is not widely stocked at electronics retailers.
