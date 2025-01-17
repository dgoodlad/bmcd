// Copyright 2023 Turing Machines
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use super::transport::{StdFwUpdateTransport, StdTransportWrapper};
use super::{FlashProgress, FlashingError, FlashingErrorExt};
use crate::firmware_update::FlashStatus;
use crate::hal::usbboot;
use anyhow::Context;
use log::info;
use rockfile::boot::{
    RkBootEntry, RkBootEntryBytes, RkBootHeader, RkBootHeaderBytes, RkBootHeaderEntry,
};
use rockusb::libusb::{Transport, TransportIO};
use rusb::DeviceDescriptor;
use rusb::GlobalContext;
use std::{mem::size_of, ops::Range, time::Duration};
use tokio::sync::mpsc::Sender;

const SPL_LOADER_RK3588: &[u8] = include_bytes!("./rk3588_spl_loader_v1.08.111.bin");

pub const RK3588_VID_PID: (u16, u16) = (0x2207, 0x350b);
pub async fn new_rockusb_transport(
    device: rusb::Device<GlobalContext>,
    logging: &Sender<FlashProgress>,
) -> Result<StdTransportWrapper<TransportIO<Transport>>, FlashingError> {
    let mut transport = Transport::from_usb_device(device.open().map_err_into_logged_usb(logging)?)
        .map_err(|_| FlashingError::UsbError)?;

    if BootMode::Maskrom
        == device
            .device_descriptor()
            .map_err_into_logged_usb(logging)?
            .into()
    {
        info!("Maskrom mode detected. loading usb-plug..");
        transport = download_boot(&mut transport, logging).await?;
        logging
            .try_send(FlashProgress {
                status: FlashStatus::Setup,
                message: format!(
                    "Chip Info bytes: {:0x?}",
                    transport
                        .chip_info()
                        .map_err_into_logged_usb(logging)?
                        .inner()
                ),
            })
            .map_err(|_| FlashingError::IoError)?;
    }

    Ok(StdTransportWrapper::new(
        transport.into_io().map_err_into_logged_io(logging)?,
    ))
}

impl StdFwUpdateTransport for TransportIO<Transport> {}

async fn download_boot(
    transport: &mut Transport,
    logging: &Sender<FlashProgress>,
) -> Result<Transport, FlashingError> {
    let boot_entries = parse_boot_entries(SPL_LOADER_RK3588).map_err_into_logged_io(logging)?;
    load_boot_entries(transport, boot_entries)
        .await
        .map_err_into_logged_io(logging)?;
    // Rockchip will reconnect to USB, back off a bit
    tokio::time::sleep(Duration::from_secs(1)).await;

    let devices = usbboot::get_usb_devices([&RK3588_VID_PID]).map_err_into_logged_usb(logging)?;
    log::debug!("re-enumerated usb devices={:?}", devices);
    assert!(devices.len() == 1);

    Transport::from_usb_device(devices[0].open().map_err_into_logged_usb(logging)?)
        .map_err_into_logged_usb(logging)
}

fn parse_boot_entries(
    raw_boot_bytes: &'static [u8],
) -> anyhow::Result<impl Iterator<Item = (u16, u32, &[u8])>> {
    let boot_header_raw = raw_boot_bytes[0..size_of::<RkBootHeaderBytes>()].try_into()?;
    let boot_header =
        RkBootHeader::from_bytes(boot_header_raw).context("Boot header loader corrupt")?;

    let entry_471 = parse_boot_header_entry(0x471, raw_boot_bytes, boot_header.entry_471)?;
    let entry_472 = parse_boot_header_entry(0x472, raw_boot_bytes, boot_header.entry_472)?;
    Ok(entry_471.chain(entry_472))
}

fn parse_boot_header_entry(
    entry_type: u16,
    blob: &[u8],
    header: RkBootHeaderEntry,
) -> anyhow::Result<impl Iterator<Item = (u16, u32, &[u8])>> {
    let mut results = Vec::new();
    let mut range = header.offset..header.offset + header.size as u32;
    for _ in 0..header.count as usize {
        let boot_entry = parse_boot_entry(blob, &range)?;
        let name = String::from_utf16(boot_entry.name.as_slice()).unwrap_or_default();
        log::debug!(
            "Found boot entry [{:x}] {} {} KiB",
            entry_type,
            name,
            boot_entry.data_size / 1024,
        );

        if boot_entry.size == 0 {
            log::debug!("skipping, size == 0 of {}", name);
            continue;
        }

        let start = boot_entry.data_offset as usize;
        let end = start + boot_entry.data_size as usize;
        let data = &blob[start..end];
        results.push((entry_type, boot_entry.data_delay, data));

        range.start += header.size as u32;
        range.end += header.size as u32;
    }

    Ok(results.into_iter())
}

fn parse_boot_entry(blob: &[u8], range: &Range<u32>) -> anyhow::Result<RkBootEntry> {
    let boot_entry_size = size_of::<RkBootEntryBytes>();
    let narrowed_range = range.start as usize..range.start as usize + boot_entry_size;
    let narrowed_slice: RkBootEntryBytes = blob[narrowed_range].try_into()?;
    Ok(RkBootEntry::from_bytes(&narrowed_slice))
}

async fn load_boot_entries(
    transport: &mut Transport,
    iterator: impl Iterator<Item = (u16, u32, &'static [u8])>,
) -> anyhow::Result<()> {
    let mut size = 0;
    for (area, delay, data) in iterator {
        transport.write_maskrom_area(area, data)?;
        tokio::time::sleep(Duration::from_millis(delay.into())).await;
        size += data.len();
    }
    log::debug!("written {} bytes", size);
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum BootMode {
    Maskrom = 0,
    Loader = 1,
}

impl From<DeviceDescriptor> for BootMode {
    fn from(dd: DeviceDescriptor) -> BootMode {
        match dd.usb_version().sub_minor() & 0x1 {
            0 => BootMode::Maskrom,
            1 => BootMode::Loader,
            _ => unreachable!(),
        }
    }
}
