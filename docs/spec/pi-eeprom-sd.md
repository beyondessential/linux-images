# Pi EEPROM-config SD artifact

A standalone, single-purpose artifact: when booted on a Raspberry Pi 5, it
flashes the EEPROM with a blessed config that enables the firmware support
we need, then reboots. It is independent of the OS images defined in
`disk-images.md` and is shipped alongside them.

## Output forms

> r[image.pi-eeprom-sd.artifact]
> The build must produce a flashable disk image.

## Bootloader configuration

> r[image.pi-eeprom-sd.bootconf]
> The EEPROM image embedded in `pieeprom.upd` must configure:
> - UART debug port enabled on boot
> - boot order: try SD if present, then PCIe (SSD), then loop
> - power off on shutdown
> - assume PSU can deliver 5A
> - any other settings that will help in the above configuration

