#![no_std]
#![no_main]

#[path = "../example_common.rs"]
mod example_common;

use core::any::Any;
use core::mem;

use cortex_m::prelude::{_embedded_hal_Pwm, _embedded_hal_PwmPin};
use defmt::{info, *};
use embassy_executor::Spawner;
use embassy_nrf::config::Config;
use embassy_nrf::gpio::{AnyPin, Output};
use embassy_nrf::Peripheral;
use embassy_nrf::interrupt::Priority;
use embassy_nrf::pwm::{self, SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Timer, Duration};
use nrf_softdevice::ble::advertisement_builder::{
    Flag, LegacyAdvertisementBuilder, LegacyAdvertisementPayload, ServiceList, ServiceUuid16,
};
use nrf_softdevice::ble::{gatt_server, peripheral, OutOfBandReply};
use nrf_softdevice::{raw, Softdevice};

#[embassy_executor::task]
async fn softdevice_task(sd: &'static Softdevice) -> ! {
    sd.run().await
}

#[nrf_softdevice::gatt_service(uuid = "180f")]
struct BatteryService {
    #[characteristic(uuid = "2a19", read, notify)]
    battery_level: u8,
}

#[nrf_softdevice::gatt_service(uuid = "9e7312e0-2354-11eb-9f10-fbc30a62cf38")]
struct FooService {
    #[characteristic(uuid = "9e7312e0-2354-11eb-9f10-fbc30a63cf38", read, write, notify, indicate)]
    foo: u16,
}

#[nrf_softdevice::gatt_server]
struct Server {
    bas: BatteryService,
    foo: FooService,
}

#[embassy_executor::task]
async fn blinker(apin: AnyPin) {
    let mut led = Output::new(
        apin,
        embassy_nrf::gpio::Level::Low,
        embassy_nrf::gpio::OutputDrive::Standard
    );
    loop {
        led.set_high();
        Timer::after(Duration::from_millis(5000)).await;
        led.set_low();
        Timer::after(Duration::from_millis(500)).await;
    }
}


#[embassy_executor::task]
async fn fader(pwmInst: impl pwm::Instance, apin: AnyPin)
{
    let seq_words: [u16; 5] = [1000, 250, 100, 50, 0];

    let mut config = pwm::Config::default();
    config.prescaler = pwm::Prescaler::Div128;
    // 1 period is 1000 * (128/16mhz = 0.000008s = 0.008ms) = 8us
    // but say we want to hold the value for 5000ms
    // so we want to repeat our value as many times as necessary until 5000ms passes
    // want 5000/8 = 625 periods total to occur, so 624 (we get the one period for free remember)
    let mut seq_config = SequenceConfig::default();
    seq_config.refresh = 624;
    // thus our sequence takes 5 * 5000ms or 25 seconds

    let mut pwm = unwrap!(SequencePwm::new_1ch(pwmInst, apin, config));
    let sequencer = SingleSequencer::new(&mut pwm, &seq_words, seq_config);
    loop {
        unwrap!(sequencer.start(SingleSequenceMode::Times(1)));

        // we can abort a sequence if we need to before its complete with pwm.stop()
        // or stop is also implicitly called when the pwm peripheral is dropped
        // when it goes out of scope
        Timer::after_millis(20000).await;
        info!("pwm stopped early!");
    }
}

fn config() -> embassy_nrf::config::Config {
    let mut config = embassy_nrf::config::Config::default();
    config.gpiote_interrupt_priority = Priority::P2;
    // it seems like setting config.time_interrupt_priority is critical...
    // https://github.com/embassy-rs/nrf-softdevice/issues/59
    config.time_interrupt_priority = Priority::P2;
    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello World!");
    // make peripheral singletons
    let p = embassy_nrf::init(config());

    // create + feed LED-GPIO task w/ red LED = P0_26
    unwrap!(spawner.spawn(blinker(
        AnyPin::from(p.P0_26)
    )));

    // create + feed LED-PWM task w/ red LED = P0_26
    unwrap!(spawner.spawn(fader(
        p.PWM0,
        AnyPin::from(p.P0_06)
    )));
    // async fn fader(apwm: Pwm, apin: AnyPin)
 

    // create soft device config
    let config = nrf_softdevice::Config {
        clock: Some(raw::nrf_clock_lf_cfg_t {
            source: raw::NRF_CLOCK_LF_SRC_RC as u8,
            rc_ctiv: 16,
            rc_temp_ctiv: 2,
            accuracy: raw::NRF_CLOCK_LF_ACCURACY_500_PPM as u8,
        }),
        conn_gap: Some(raw::ble_gap_conn_cfg_t {
            conn_count: 6,
            event_length: 24,
        }),
        conn_gatt: Some(raw::ble_gatt_conn_cfg_t { att_mtu: 256 }),
        gatts_attr_tab_size: Some(raw::ble_gatts_cfg_attr_tab_size_t {
            attr_tab_size: raw::BLE_GATTS_ATTR_TAB_SIZE_DEFAULT,
        }),
        gap_role_count: Some(raw::ble_gap_cfg_role_count_t {
            adv_set_count: 1,
            periph_role_count: 3,
            central_role_count: 3,
            central_sec_count: 0,
            _bitfield_1: raw::ble_gap_cfg_role_count_t::new_bitfield_1(0),
        }),
        gap_device_name: Some(raw::ble_gap_cfg_device_name_t {
            p_value: b"HelloRust" as *const u8 as _,
            current_len: 9,
            max_len: 9,
            write_perm: unsafe { mem::zeroed() },
            _bitfield_1: raw::ble_gap_cfg_device_name_t::new_bitfield_1(raw::BLE_GATTS_VLOC_STACK as u8),
        }),
        ..Default::default()
    };

    // start softdevice
    let sd = Softdevice::enable(&config);
    let server = unwrap!(Server::new(sd));
    unwrap!(spawner.spawn(softdevice_task(sd)));

    static ADV_DATA: LegacyAdvertisementPayload = LegacyAdvertisementBuilder::new()
        .flags(&[Flag::GeneralDiscovery, Flag::LE_Only])
        .services_16(ServiceList::Complete, &[ServiceUuid16::BATTERY])
        .full_name("HelloRust")
        .build();

    static SCAN_DATA: LegacyAdvertisementPayload = LegacyAdvertisementBuilder::new()
        .services_128(
            ServiceList::Complete,
            &[0x9e7312e0_2354_11eb_9f10_fbc30a62cf38_u128.to_le_bytes()],
        )
        .build();

    loop {
        let config = peripheral::Config::default();
        let adv = peripheral::ConnectableAdvertisement::ScannableUndirected {
            adv_data: &ADV_DATA,
            scan_data: &SCAN_DATA,
        };
        let conn = unwrap!(peripheral::advertise_connectable(sd, adv, &config).await);

        info!("advertising done!");

        // Run the GATT server on the connection. This returns when the connection gets disconnected.
        //
        // Event enums (ServerEvent's) are generated by nrf_softdevice::gatt_server
        // proc macro when applied to the Server struct above
        let e = gatt_server::run(&conn, &server, |e| match e {
            ServerEvent::Bas(e) => match e {
                BatteryServiceEvent::BatteryLevelCccdWrite { notifications } => {
                    info!("battery notifications: {}", notifications)
                }
            },
            ServerEvent::Foo(e) => match e {
                FooServiceEvent::FooWrite(val) => {
                    info!("wrote foo: {}", val);
                    if let Err(e) = server.foo.foo_notify(&conn, &(val + 1)) {
                        info!("send notification error: {:?}", e);
                    }
                }
                FooServiceEvent::FooCccdWrite {
                    indications,
                    notifications,
                }
                => {
                    info!("foo indications: {}, notifications: {}", indications, notifications)
                }
            },
        })
        .await;

        info!("gatt_server run exited with error: {:?}", e);
    }
}
