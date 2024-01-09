// Copyright Cartesi and individual authors (see AUTHORS)
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

use std::fs::{File, OpenOptions};
use std::io::{SeekFrom, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::sync::Arc;

use async_mutex::Mutex;
use nix::ioctl_readwrite;
use tokio::process::Command;

const EXCEPTION: u64 = 0x00002;
#[repr(C)]
pub struct YieldRequest {
    dev: u8,
    cmd: u8,
    reason: u16,
    data: u32,
}
ioctl_readwrite!(ioctl_yield, 0xd1, 0, YieldRequest);


const HTIF_DEVICE_YIELD: u8 = 2;
const HTIF_YIELD_AUTOMATIC: u8 = 0;
const HTIF_YIELD_MANUAL: u8 = 1;
const HTIF_YIELD_REASON_PROGRESS: u16 = 0;
const HTIF_YIELD_REASON_EXCEPTION: u16 = 6;

fn do_yield(reason: u16) {

    let file = File::open("/dev/yield").unwrap();
    let fd = file.as_raw_fd();

    let mut data = YieldRequest {
        dev: HTIF_DEVICE_YIELD,
        cmd: HTIF_YIELD_MANUAL,
        reason,
        data: 0,
    };

    unsafe {
        ioctl_yield(fd, &mut data).unwrap();
    }
}
pub async fn do_exception(why: &str) {
    let mut file = OpenOptions::new()
    .write(true)
    .open(std::env::var("IO_DEVICE").unwrap()).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write(&EXCEPTION.to_be_bytes()).unwrap();

    let exception_data = why.as_bytes();

    let exception_length = why. len() as u64;
    file.seek(SeekFrom::Start(8)).unwrap();
    file.write(&exception_length.to_be_bytes()).unwrap();

    file.seek(SeekFrom::Start(16)).unwrap();
    file.write(&exception_data).unwrap();
    file.sync_all().unwrap();

    do_yield(HTIF_YIELD_REASON_EXCEPTION);
}

/// Execute the dapp command and throw a rollup exception if it fails or exits
pub async fn run(args: Vec<String>) {
    log::info!("starting dapp: {}", args.join(" "));
    let task = tokio::task::spawn_blocking(move || Command::new(&args[0]).args(&args[1..]).spawn());
    let message = match task.await {
        Ok(command_result) => match command_result {
            Ok(mut child) => match child.wait().await {
                Ok(status) => format!("dapp exited with {}", status),
                Err(e) => format!("dapp wait failed with {}", e),
            },
            Err(e) => format!("dapp failed to start with {}", e),
        },
        Err(e) => format!("failed to spawn task with {}", e),
    };
    log::warn!("throwing exception because {}", message);
    do_exception(&message).await;
}
