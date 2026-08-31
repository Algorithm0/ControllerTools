use crate::controller::Status;

use super::bluetooth::{get_battery_percentage, get_bluetooth_address};
use dbus::blocking::Connection;
use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
use dbus::Path;
use std::time::Duration;
use anyhow::Result;
use hidapi::{DeviceInfo, HidApi};
use log::error;

use super::Controller;

pub const MS_VENDOR_ID: u16 = 0x045e;

// Xbox One S controller
pub const XBOX_ONE_S_CONTROLLER_USB_PRODUCT_ID: u16 = 0x02ea; // 746
pub const XBOX_ONE_S_CONTROLLER_BT_PRODUCT_ID: u16 = 0x02df; // 765

// after upgrade to the latest firmware (same as Series X/S),
// the One S controller changed product ID!
pub const XBOX_ONE_S_LATEST_FW_PRODUCT_ID: u16 = 0x0b20; // 2848

// Xbox Wireless Controller (model 1914)
pub const XBOX_WIRELESS_CONTROLLER_USB_PRODUCT_ID: u16 = 0x0b12; // 2834
pub const XBOX_WIRELESS_CONTROLLER_BT_PRODUCT_ID: u16 = 0x0b13; // 2835

// Xbox Elite Wireless Controller Series 2
pub const XBOX_WIRELESS_ELITE_CONTROLLER_USB_PRODUCT_ID: u16 = 0x0b00;
pub const XBOX_WIRELESS_ELITE_CONTROLLER_BT_PRODUCT_ID: u16 = 0x0b05;
pub const XBOX_WIRELESS_ELITE_CONTROLLER_BTLE_PRODUCT_ID: u16 = 0x0b22;
// Xbox Accessory
pub const XBOX_ACCESSORY_PID: u16 = 0x02fe; // New accessory PID
// pub const XBOX_ONE_REPORT_BT_SIZE: usize = 64;

fn get_xbox_controller_name(product_id: u16) -> &'static str {
    match product_id {
        XBOX_ONE_S_CONTROLLER_USB_PRODUCT_ID => "Xbox One S",
        XBOX_ONE_S_CONTROLLER_BT_PRODUCT_ID => "Xbox One S",
        XBOX_ONE_S_LATEST_FW_PRODUCT_ID => "Xbox One S",
        XBOX_WIRELESS_CONTROLLER_USB_PRODUCT_ID => "Xbox Series X/S",
        XBOX_WIRELESS_CONTROLLER_BT_PRODUCT_ID => "Xbox Series X/S",
        XBOX_WIRELESS_ELITE_CONTROLLER_USB_PRODUCT_ID => "Xbox Elite 2",
        XBOX_WIRELESS_ELITE_CONTROLLER_BT_PRODUCT_ID => "Xbox Elite 2",
        XBOX_WIRELESS_ELITE_CONTROLLER_BTLE_PRODUCT_ID => "Xbox Elite 2",
        XBOX_ACCESSORY_PID => "Wireless Adapter",
        _ => "Xbox Unknown",
    }
}

pub fn is_xbox_controller(vendor_id: u16) -> bool {
    vendor_id == MS_VENDOR_ID
}

pub fn update_xbox_controller(controller: &mut Controller, bluetooth: bool) {

    controller.name = get_xbox_controller_name(controller.product_id).to_string();
    controller.capacity = if controller.gip.starts_with("gip") {
        get_battery_percentage_for_gip(&controller.gip)
    } else if bluetooth {
        0
    } else {
        100
    };
    //controller.capacity = if bluetooth { 0 } else { 99 }; // for now for USB, "fake" it and set capacity to 100 as charging

    controller.status =
    if controller.gip.starts_with("gip")
    {Status::Unknown}
    else if bluetooth {
        Status::Unknown
    } else {
        // for now for USB, "fake" it and set status to charging since it's plugged in
        Status::Charging
    };
 }

pub fn parse_xbox_controller_data(
    device_info: &DeviceInfo,
    _hidapi: &HidApi,
) -> Result<Controller> {
    let capacity: u8 = match get_bluetooth_address(device_info) {
        Ok(address) => match get_battery_percentage(address) {
            Ok(percentage) => percentage,
            Err(err) => {
                error!("get_battery_percentage failed because {}", err);
                0
            }
        },
        Err(err) => {
            error!("get_bluetooth_address failed because {}", err);
            0
        }
    };
    let name = get_xbox_controller_name(device_info.product_id());

    let controller = Controller::from_hidapi(device_info, name, capacity, Status::Unknown);
    Ok(controller)
}

fn get_battery_percentage_for_gip(gip: &str) -> u8 {
    // Normalize the `gip` to match UPower paths
    let normalized_gip = format!("battery_{}", gip.replace(".", "x"));

    // Create a DBus connection
    let connection = match Connection::new_system() {
        Ok(conn) => conn,
        Err(err) => {
            log::error!("Failed to connect to DBus: {}", err);
            return 10;
        }
    };

    // Proxy to UPower
    let proxy = connection.with_proxy(
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        Duration::from_millis(5000),
    );

    // Enumerate devices
    let (devices,): (Vec<Path>,) = match proxy.method_call(
        "org.freedesktop.UPower",
        "EnumerateDevices",
        (),
    ) {
        Ok(devices) => devices,
        Err(err) => {
            log::error!("Failed to enumerate devices: {}", err);
            return 20;
        }
    };

    // Iterate through devices to find the matching `gip`
    for device_path in devices {
        let device_path_str = device_path.to_string();
        if let Some(upower_gip) = device_path_str.split('/').find(|&s| s.starts_with("battery_")) {
            if upower_gip == normalized_gip {
                // Found matching `gip`, query percentage
                let device_proxy = connection.with_proxy(
                    "org.freedesktop.UPower",
                    device_path.clone(),
                                                         Duration::from_millis(5000),
                );

                return match device_proxy.get::<f64>("org.freedesktop.UPower.Device", "Percentage") {
                    Ok(percentage) => percentage as u8,
                    Err(err) => {
                        log::error!("Failed to get battery percentage for {}: {}", device_path_str, err);
                        0
                    }
                };
            }
        }
    }

    0 // Return 0 if no match is found
}
