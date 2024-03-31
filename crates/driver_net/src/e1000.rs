use core::convert::From;
use core::{mem::ManuallyDrop, ptr::NonNull};
use alloc::boxed::Box;
use alloc::vec::Vec;

use alloc::{collections::VecDeque, sync::Arc};
use driver_common::{BaseDriverOps, DevError, DevResult, DeviceType};
use e1000_driver::e1000::E1000Device;

pub use e1000_driver::e1000::KernelFunc;
use crate::{EthernetAddress, NetBufBox, NetBufPtr, NetDriverOps};

extern crate alloc;

const RECV_BATCH_SIZE: usize = 64;
const RX_BUFFER_SIZE: usize = 1024;

pub struct E1000Nic<'a, K: KernelFunc> {
    inner: E1000Device<'a, K>,
    free_tx_bufs: Vec<NetBufBox>,
}
use log::info;

unsafe impl<'a, K: KernelFunc> Sync for E1000Nic<'a, K> {}
unsafe impl<'a, K: KernelFunc> Send for E1000Nic<'a, K> {}

impl<'a, K: KernelFunc> E1000Nic<'a, K> {
    pub fn init(mut kfn: K, mapped_regs: usize) -> DevResult<Self> {
        info!("E1000Nic init");
        Ok(Self {
            inner: E1000Device::<K>::new(kfn, mapped_regs).map_err(|err| {
                log::error!("Failed to initialize e1000 device: {:?}", err);
                DevError::BadState
            })?,
            free_tx_bufs: Vec::with_capacity(RX_BUFFER_SIZE),
        })
    }
}

impl<'a, K: KernelFunc> BaseDriverOps for E1000Nic<'a, K> {
    fn device_name(&self) -> &str {
        "e1000:Intel 82540EP/EM"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Net
    }
}

impl<'a, K: KernelFunc> NetDriverOps for E1000Nic<'a, K> {
    fn mac_address(&self) -> EthernetAddress {
        info!("E1000 get mac address");
        EthernetAddress([0x00, 0x0c, 0x29, 0x3e, 0x4f, 0x50])
    }

    fn rx_queue_size(&self) -> usize {
        256
    }

    fn tx_queue_size(&self) -> usize {
        256
    }

    fn can_receive(&self) -> bool {
        true
    }

    fn can_transmit(&self) -> bool {
        true
    }

    fn recycle_rx_buffer(&mut self, rx_buf: NetBufPtr) -> DevResult {
        drop(rx_buf);
        Ok(())
    }

    fn recycle_tx_buffers(&mut self) -> DevResult {
        Ok(())
    }

    fn receive(&mut self) -> DevResult<NetBufPtr> {
        info!("E1000Nic receive");
        match self.inner.e1000_recv() {
            None => Err(DevError::Again),
            Some(packets) => {
                let total_len = packets.iter().map(|p| p.len()).sum();
                let mut buf = Box::new(Vec::<u8>::with_capacity(total_len));
                let mut offset = 0;
                for packet in packets {
                    buf[offset..offset + packet.len()].copy_from_slice(&packet);
                    offset += packet.len();
                }
                info!("E1000Nic receive end");
                Ok(NetBufPtr::new(NonNull::dangling(), NonNull::new(Box::into_raw(buf) as *mut u8).unwrap(), total_len))
            },
        }
    }

    fn transmit(&mut self, tx_buf: NetBufPtr) -> DevResult {
        info!("E1000Nic transmit");
        self.inner.e1000_transmit(tx_buf.packet());
        info!("E1000Nic transmit end");
        Ok(())
    }

    fn alloc_tx_buffer(&mut self, size: usize) -> DevResult<NetBufPtr> {
        info!("E1000Nic alloc_tx_buffer");
        // 0. Allocate a buffer from the queue.
        let mut net_buf = self.free_tx_bufs.pop().ok_or(DevError::NoMemory)?;
        let pkt_len = size;

        // 1. Check if the buffer is large enough.
        let hdr_len = net_buf.header_len();
        if hdr_len + pkt_len > net_buf.capacity() {
            return Err(DevError::InvalidParam);
        }
        net_buf.set_packet_len(pkt_len);
        info!("E1000Nic alloc_tx_buffer end");

        // 2. Return the buffer.
        Ok(net_buf.into_buf_ptr())
    }
}
