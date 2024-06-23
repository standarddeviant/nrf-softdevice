MEMORY
{
  /* NOTE 1 K = 1 KiBi = 1024 bytes */
  /* NRF52832 with Softdevice S112 7.x and 6.x */
  FLASH : ORIGIN = 0x00019000, LENGTH = 192K - 100K
  RAM : ORIGIN = 0x20000000 + 4K, LENGTH = 64K - 4K
}
