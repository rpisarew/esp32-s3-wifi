#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

extern crate alloc;
#[global_allocator]
static ALLOCATOR: esp_alloc::EspHeap = esp_alloc::EspHeap::empty();
use core::mem::MaybeUninit;

use esp_wifi::{
    initialize,
    wifi::{WifiController, WifiDevice, WifiEvent, WifiStaDevice, WifiState},
    EspWifiInitFor,
};

use esp32s3_hal as hal;
use hal::{
    clock::ClockControl,
    embassy::{self},
    peripherals::Peripherals,
    prelude::*,
    Rng,
};

use embassy_executor::Spawner;
use embassy_net::{Config, Stack, StackResources};
use embassy_time::{Duration, Timer};
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};

use esp_backtrace as _;
use static_cell::make_static;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

fn init_heap() {
    const HEAP_SIZE: usize = 128 * 1024;
    static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

    unsafe {
        ALLOCATOR.init(HEAP.as_mut_ptr() as *mut u8, HEAP_SIZE);
        log::trace!("heap initialized");
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await;
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    log::info!(
        "[WC] device capabilities: {:?}",
        controller.get_capabilities()
    );

    loop {
        if let WifiState::StaConnected = esp_wifi::wifi::get_wifi_state() {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5_000)).await
        }

        let mut ssid = heapless::String::new();
        for c in SSID.chars() {
            ssid.push(c).unwrap();
        }

        let mut password = heapless::String::new();
        for c in PASSWORD.chars() {
            password.push(c).unwrap();
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid,
                password,
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            log::info!("[WC] starting..");
            Timer::after(Duration::from_millis(5_000)).await;
            controller.start().await.unwrap(); // here it stops executing other tasks..
            log::info!("[WC] started!");
        }

        log::info!("[WC] connecting to '{SSID}'");

        match controller.connect().await {
            Ok(_) => log::info!("[WC] connection with '{SSID}' is up!"),
            Err(e) => {
                log::warn!("[WC] connection with '{SSID}' failed: {e:?}");
                Timer::after(Duration::from_millis(1_000)).await
            }
        }
    }
}

#[main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    init_heap();

    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    // #[cfg(feature = "embassy-time-systick")]
    // {
    //     embassy::init(
    //         &clocks,
    //         hal::systimer::SystemTimer::new(peripherals.SYSTIMER),
    //     );
    //     log::trace!("embassy initialized");
    // }

    // #[cfg(feature = "embassy-time-timg0")]
    // {
    embassy::init(
        &clocks,
        hal::timer::TimerGroup::new(peripherals.TIMG0, &clocks),
    );
    esp_println::println!("embassy::init embassy-time-timg0");
    // }

    let init = initialize(
        EspWifiInitFor::Wifi,
        hal::timer::TimerGroup::new(peripherals.TIMG1, &clocks).timer0,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let config = Config::dhcpv4(Default::default());
    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*make_static!(Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<3>::new()),
        seed
    ));

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        log::info!("wait for link");
        Timer::after(Duration::from_millis(1_000)).await;
    }

    log::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            log::info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(1_000)).await;
    }

    loop {
        log::info!("Main Loop");
        Timer::after(Duration::from_millis(1_000)).await;
    }
}
