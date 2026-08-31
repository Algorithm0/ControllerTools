mod bluetooth;
mod generic;
mod nintendo;
mod playstation;
mod xbox;
use anyhow::Result;
use hidapi::HidApi;
use log::debug;
use udev::Enumerator;
use std::collections::{HashMap, HashSet};

use crate::controller::{Controller, Status};

pub async fn controllers_async() -> Result<Vec<Controller>> {
    // Spawn a tokio blocking task because `get_controllers()` is a blocking API
    let controllers = tokio::task::spawn_blocking(controllers).await??;
    Ok(controllers)
}

fn rename_duplicate_controllers(vec: &mut Vec<Controller>) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for c in vec.iter() {
        *counts.entry(c.name.clone()).or_insert(0) += 1;
    }

    for c in vec.iter_mut() {
        if counts.get(&c.name).copied().unwrap_or(0) >= 2 {
            if c.uniq.is_empty() {
                continue;
            }

            let suffix: String = if c.uniq.chars().count() <= 4 {
                c.uniq.clone()
            } else {
                c.uniq
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<char>>()
                    .into_iter()
                    .rev()
                    .collect()
            };

            c.name = format!("{} ({})", c.name.replace("Xbox ", "").replace("Microsoft ", ""), suffix);
        }
    }
}

pub fn controllers() -> Result<Vec<Controller>> {
    let hidapi = HidApi::new()?;
    let mut controllers: Vec<Controller> = Vec::new();

    // If in debug mode, check if there is a fake controller in /tmp/fake_controller.json
    if cfg!(debug_assertions) {
        parse_fake_controller(&mut controllers);
    }

    // HidApi will return 2 copies of the device when the Nintendo Pro Controller is connected via USB.
    // It will additionally return a 3rd device when the controller is connected via Bluetooth + USB.
    let nintendo_pro_controllers: Vec<_> = hidapi
        .device_list()
        .filter(|device_info| {
            device_info.vendor_id() == nintendo::VENDOR_ID_NINTENDO
                && device_info.product_id() == nintendo::PRODUCT_ID_NINTENDO_PROCON
        })
        .collect();

    if nintendo_pro_controllers.len() == 1 || nintendo_pro_controllers.len() == 2 {
        // When we only get one device, we know it's connected via Bluetooth.
        // When we get two devices, we know it's connected only via USB. Both will report the same data, so we'll just return the first one.
        let controller = nintendo::parse_controller_data(nintendo_pro_controllers[0], &hidapi)?;
        controllers.push(controller);
    } else if nintendo_pro_controllers.len() == 3 {
        // When we get three devices, we know it's connected via USB + Bluetooth.
        // We'll only return the Bluetooth device because the USB devices will not report any data.
        let bt_controller = nintendo_pro_controllers
            .iter()
            .find(|device_info| device_info.interface_number() == -1);

        if let Some(bt_controller) = bt_controller {
            let controller = nintendo::parse_controller_data(bt_controller, &hidapi)?;
            controllers.push(controller);
        }
    }

    let nintendo_non_pro_controllers: Vec<_> = hidapi
        .device_list()
        .filter(|device_info| {
            device_info.vendor_id() == nintendo::VENDOR_ID_NINTENDO
                && device_info.product_id() != nintendo::PRODUCT_ID_NINTENDO_PROCON
        })
        .collect();
    for device_info in nintendo_non_pro_controllers {
        let controller = nintendo::parse_controller_data(device_info, &hidapi)?;
        controllers.push(controller);
    }

    // for some reason HidApi's list_devices() is returning multiple instances of the same controller
    // so dedupe by serial number
    let mut xbox_controllers: Vec<_> = hidapi
        .device_list()
        .filter(|device_info| {
            device_info.vendor_id() == xbox::MS_VENDOR_ID
                && (device_info.product_id() == xbox::XBOX_ONE_S_CONTROLLER_BT_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_ONE_S_LATEST_FW_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_WIRELESS_CONTROLLER_USB_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_WIRELESS_CONTROLLER_BT_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_WIRELESS_ELITE_CONTROLLER_USB_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_WIRELESS_ELITE_CONTROLLER_BT_PRODUCT_ID
                    || device_info.product_id() == xbox::XBOX_WIRELESS_ELITE_CONTROLLER_BTLE_PRODUCT_ID)
        })
        .collect();
    xbox_controllers.dedup_by(|a, b| a.serial_number() == b.serial_number());
    for device_info in xbox_controllers {
        match (device_info.vendor_id(), device_info.product_id()) {
            (xbox::MS_VENDOR_ID, xbox::XBOX_ONE_S_CONTROLLER_BT_PRODUCT_ID) => {
                debug!("!Found Xbox One S controller: {:?}", device_info);
                let controller = xbox::parse_xbox_controller_data(device_info, &hidapi)?;
                controllers.push(controller);
            }
            (xbox::MS_VENDOR_ID, xbox::XBOX_ONE_S_LATEST_FW_PRODUCT_ID) => {
                debug!("Found Xbox One S controller: {:?}", device_info);
                let controller = xbox::parse_xbox_controller_data(device_info, &hidapi)?;

                controllers.push(controller);
            }
            (xbox::MS_VENDOR_ID, xbox::XBOX_WIRELESS_CONTROLLER_BT_PRODUCT_ID) => {
                debug!("Found Xbox Series X/S controller: {:?}", device_info);
                let controller = xbox::parse_xbox_controller_data(device_info, &hidapi)?;
                controllers.push(controller);
            }
            (xbox::MS_VENDOR_ID, xbox::XBOX_WIRELESS_ELITE_CONTROLLER_BT_PRODUCT_ID) => {
                debug!("Found Xbox Elite 2 controller: {:?}", device_info);
                let controller = xbox::parse_xbox_controller_data(device_info, &hidapi)?;
                controllers.push(controller);
            }
            (xbox::MS_VENDOR_ID, xbox::XBOX_WIRELESS_ELITE_CONTROLLER_BTLE_PRODUCT_ID) => {
                debug!("Found Xbox Elite 2 controller: {:?}", device_info);
                let controller = xbox::parse_xbox_controller_data(device_info, &hidapi)?;
                controllers.push(controller);
            }
            _ => {}
        }
    }

    let mut unique_devices: Vec<_> = hidapi.device_list().collect();
    unique_devices.dedup_by(|a, b| a.serial_number() == b.serial_number());
    for device_info in unique_devices {
        match (device_info.vendor_id(), device_info.product_id()) {
            (playstation::DS_VENDOR_ID, playstation::DS3_PRODUCT_ID) => {
                debug!("Found DualShock3 controller: {:?}", device_info);
                let controller = playstation::parse_dualshock3_controller_data(
                    device_info,
                    &hidapi,
                    "DualShock3",
                )?;

                controllers.push(controller);
            }
            (playstation::DS_VENDOR_ID, playstation::DS_PRODUCT_ID) => {
                debug!("Found DualSense controller: {:?}", device_info);
                let controller = playstation::parse_dualsense_controller_data(
                    device_info,
                    &hidapi,
                    "DualSense",
                )?;

                controllers.push(controller);
            }
            (playstation::DS_VENDOR_ID, playstation::DS_EDGE_PRODUCT_ID) => {
                debug!("Found DualSense Edge controller: {:?}", device_info);
                let controller = playstation::parse_dualsense_controller_data(
                    device_info,
                    &hidapi,
                    "DualSense Edge",
                )?;

                controllers.push(controller);
            }
            (playstation::DS_VENDOR_ID, playstation::DS4_ADAPTER_PRODUCT_ID) => {
                debug!("Found new DualShock 4 controller: {:?}", device_info);
                let controller =
                    playstation::parse_dualshock_controller_data(device_info, &hidapi)?;

                controllers.push(controller);
            }
            (playstation::DS_VENDOR_ID, playstation::DS4_NEW_PRODUCT_ID) => {
                debug!("Found new DualShock 4 controller: {:?}", device_info);
                let controller =
                    playstation::parse_dualshock_controller_data(device_info, &hidapi)?;

                controllers.push(controller);
            }
            (playstation::DS_VENDOR_ID, playstation::DS4_OLD_PRODUCT_ID) => {
                debug!("Found old DualShock 4 controller: {:?}", device_info);
                let controller =
                    playstation::parse_dualshock_controller_data(device_info, &hidapi)?;

                controllers.push(controller);
            }
            _ => {}
        }
    }

    let mut enumerator = Enumerator::new()?;
    enumerator.match_subsystem("input")?;

    let mut seen_gips_map = HashMap::new();

    for device in enumerator.scan_devices()? {
        let mut controller = Controller::from_udev(&device, 0, Status::Unknown, false);

        // Only include records where gip starts with "gip" or "input" and exclude "gip0.1"
        if !(controller.gip.starts_with("gip") || controller.gip.starts_with("input")) || controller.gip == "gip0.1"  {
            continue;
        }

        if !xbox::is_xbox_controller(controller.vendor_id) {
            continue;
        }

        let iter = seen_gips_map.get(&controller.gip);
        if iter.is_none() {
            xbox::update_xbox_controller(&mut controller, false);
            seen_gips_map.insert(controller.gip, controller);
        } else if controller.name != Controller::NO_NAME
                && !controller.name.is_empty() && iter.unwrap().name != controller.name
        {
            xbox::update_xbox_controller(&mut controller, false);
            seen_gips_map.remove(&controller.gip);
            seen_gips_map.insert(controller.gip, controller);
        }
    }

    let mut vec: Vec<Controller> = seen_gips_map.values().cloned().collect();
    rename_duplicate_controllers(&mut vec);

    Ok(vec)
}

fn parse_fake_controller(controllers: &mut Vec<Controller>) {
    if let Ok(file) = std::fs::File::open("/tmp/fake_controller.json") {
        let controller = match serde_json::from_reader(file) {
            Ok(controller) => {
                debug!("Loaded fake controller: {:?}", controller);
                Some(controller)
            }
            Err(e) => {
                debug!("Error parsing fake controller: {}", e);
                None
            }
        };
        if let Some(controller) = controller {
            controllers.push(controller);
        }
    }
}
