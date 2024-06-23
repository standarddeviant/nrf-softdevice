#![no_std]
#![no_main]

#[path = "../example_common.rs"]
mod example_common;

use core::mem;

use defmt::{info, *};
use {defmt_rtt as _, panic_probe as _};

use embassy_executor::Spawner;
// use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::{Channel, Sender};

use embassy_nrf::gpio::{AnyPin, Input, Output, Pull};
use embassy_nrf::interrupt::Priority;
use embassy_time::{Duration, Instant, Timer};
use nrf_softdevice::ble::advertisement_builder::{
    Flag, LegacyAdvertisementBuilder, LegacyAdvertisementPayload, ServiceList, ServiceUuid16,
};
use nrf_softdevice::ble::{gatt_server, peripheral};
use nrf_softdevice::{raw, Softdevice};

const BUTTON_CHANNEL_SIZE: usize = 8;

#[derive(Clone, Copy)]
enum ButtonState {
    Pressed,
    Released,
}

// // button debounce impl
// pub struct Debouncer<'a> {
//     input: Input<'a>,
//     debounce: Duration,
// }

// impl<'a> Debouncer<'a> {
//     pub fn new(input: Input<'a>, debounce: Duration) -> Self {
//         Self { input, debounce }
//     }

//     pub async fn debounce(&mut self) -> Level {
//         loop {
//             let l1 = self.input.get_level();

//             self.input.wait_for_any_edge().await;

//             Timer::after(self.debounce).await;

//             let l2 = self.input.get_level();
//             if l1 != l2 {
//                 break l2;
//             }
//         }
//     }
// }

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
async fn led_task(apin: AnyPin) {
    let mut led = Output::new(
        apin,
        embassy_nrf::gpio::Level::Low,
        embassy_nrf::gpio::OutputDrive::Standard,
    );
    loop {
        led.set_high();
        Timer::after(Duration::from_millis(500)).await;
        led.set_low();
        Timer::after(Duration::from_millis(500)).await;
    }
}

// NOTE: PubSubChannel has 'generic's
// M = Mutex
// T = Type
// then...
// CAPS, SUBS, and PUBS
// So (5) 'generic's in total

#[embassy_executor::task]
async fn button_task(
    sender: Sender<'static, ThreadModeRawMutex, (Instant, ButtonState), BUTTON_CHANNEL_SIZE>,
    mut btn: Input<'static, AnyPin>, // debounce_dur: Duration
) {
    loop {
        btn.wait_for_low().await;
        sender.send((Instant::now(), ButtonState::Pressed)).await;
        info!("Button pressed!");

        btn.wait_for_high().await;
        sender.send((Instant::now(), ButtonState::Released)).await;
        info!("Button released!");
    }
}

fn nrf_config() -> embassy_nrf::config::Config {
    let mut config = embassy_nrf::config::Config::default();
    config.gpiote_interrupt_priority = Priority::P2;
    // it seems like setting config.time_interrupt_priority is critical...
    // https://github.com/embassy-rs/nrf-softdevice/issues/59
    config.time_interrupt_priority = Priority::P2;
    config
}

static BUTTON_CHANNEL: Channel<ThreadModeRawMutex, (Instant, ButtonState), BUTTON_CHANNEL_SIZE> = Channel::new();
// static LED_CHANNEL: Channel<ThreadModeRawMutex, u32, LED_CHANNEL_SIZE> = Channel::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello World!");
    // make peripheral singletons
    let p = embassy_nrf::init(nrf_config());

    // 1. Spawn LED Task w/ red LED = P0_26
    unwrap!(spawner.spawn(led_task(AnyPin::from(p.P0_26))));

    // 2. Spawn Button task w/ ???
    let button = Input::new(AnyPin::from(p.P0_12), Pull::Up);
    // let mut btn_pubsub = PubSubChannel::<NoopRawMutex, (Instant, ButtonState), 4, 4, 4>::new();
    unwrap!(spawner.spawn(button_task(
        BUTTON_CHANNEL.sender(),
        button // Duration::from_millis(20)
    )));

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
                } => {
                    info!("foo indications: {}, notifications: {}", indications, notifications)
                }
            },
        })
        .await;

        info!("gatt_server run exited with error: {:?}", e);
    }
}
