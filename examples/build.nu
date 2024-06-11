
def main [] {

  let INFO = $"\n(ansi green_bold)INFO:"

  # read src/bin 
  let src_bin_list = (ls ([. src bin *rs] | path join)).name
  let select_list = (
    $src_bin_list | each {
      |x|
      $x | (parse -r '(?P<noext>\w+)\.rs').noext.0
    }
  )
  echo $select_list
  echo $select_list.0

  let name = $select_list | input list 'Select which bin to build'
  let bpath = [.. target thumbv7em-none-eabihf release $name] | path join
  let hpath = [. $'($name).hex'] | path join

  echo $'($INFO) Building ($name) ...(ansi reset)'
  cargo build --bin $name --features nrf52840 --release

  echo $'($INFO)  converting bin-to-hex ...(ansi reset)'
  arm-none-eabi-objcopy -O ihex $bpath $hpath

  echo $'($INFO) merging softdevice + app hex files ...(ansi reset)'
  mergehex -m s140_nrf52_7.3.0_softdevice.hex $hpath -o merged.hex

  echo $'($INFO) converting merged .hex to merged .uf2 file ...(ansi reset)'
  python uf2conv.py -f 0xADA52840 -c -b 0x1000 -o sd_app.uf2 merged.hex
}

