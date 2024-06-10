
cargo build --bin ble_bas_peripheral_blinky --features nrf52840 --release
arm-none-eabi-objcopy -O ihex ..\target\thumbv7em-none-eabihf\release\ble_bas_peripheral_blinky .\ble_bas_peripheral_blinky.hex
mergehex -m s140_nrf52_7.3.0_softdevice.hex ble_bas_peripheral.hex -o merged.hex
python uf2conv.py -f 0xADA52840 -c -b 0x1000 -o app.uf2 merged.hex

