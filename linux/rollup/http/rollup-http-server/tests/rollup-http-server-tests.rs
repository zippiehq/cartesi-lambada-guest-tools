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

extern crate rollup_http_client;
extern crate rollup_http_server;

use actix_server::ServerHandle;
use async_mutex::Mutex;
use futures::StreamExt;
use rollup_http_client::rollup::{
    Exception, Notice, Report, RollupRequest, RollupResponse, Voucher
};
use rollup_http_server::config::Config;
use rollup_http_server::*;
use rstest::*;
use std::error::Error;
use std::fs::File;
use std::future::Future;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::sync::Arc;

use std::os::fd::FromRawFd;
use std::io::SeekFrom;
use nix::fcntl::OFlag;
use std::io::Seek;
use std::io::Read;
use std::io::Write;
use ipfs_api_backend_hyper::{IpfsApi, IpfsClient, TryFromUri};
use std::io::Cursor;
use actix_web::web::BytesMut;

#[repr(align(4096))]
struct Aligned([u8; 4096 as usize]);

use std::{
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt,
};
const PORT: u16 = 10010;
const HOST: &str = "127.0.0.1";
const IPFS_PORT: u16 = 5001;
const TEST_ROLLUP_DEVICE: &str = "rollup_driver.bin";

#[allow(dead_code)]
struct Context {
    address: String,
    server_handle: actix_server::ServerHandle,
}

impl Drop for Context {
    fn drop(&mut self) {
        // Shut down http server+
        println!("shutting down http service in drop cleanup");
    }
}

fn run_test_http_service(
    host: &str,
    port: u16,
) -> std::io::Result<Option<actix_server::ServerHandle>> {
    println!("Opening rollup device");
    // Open test rollup device
    let rollup_file = match File::create(TEST_ROLLUP_DEVICE) {
        Ok(file) => file,
        Err(e) => {
            log::error!("error opening rollup device {}", e.to_string());
            return Err(e);
        }
    };

    let rollup_fd: Arc<Mutex<RawFd>> = Arc::new(Mutex::new(rollup_file.into_raw_fd()));
    let rollup_fd = rollup_fd.clone();
    let http_config = Config {
        http_address: host.to_string(),
        http_port: port,
    };
    println!("Creating http server");
    let server = http_service::create_server(&http_config)?;
    let server_handle = server.handle();
    println!("Spawning http server");
    tokio::spawn(server);
    println!("Http server spawned");
    Ok(Some(server_handle))
}

#[fixture]
async fn context_future() -> Context {
    let mut server_handle: Option<ServerHandle> = None;
    let mut count = 5;
    loop {
        match run_test_http_service(HOST, PORT) {
            Ok(handle) => {
                server_handle = handle;
                break;
            }
            Err(ex) => {
                eprint!("Error instantiating rollup http service {}", ex.to_string());
                if count > 0 {
                    // wait for the system to free port
                    std::thread::sleep(std::time::Duration::from_secs(1));
                } else {
                    break;
                }
            }
        };
        count = count - 1;
    }

    Context {
        address: format!("http://{}:{}", HOST, PORT),
        server_handle: server_handle.unwrap(),
    }
}

#[rstest]
#[tokio::test]
async fn test_server_instance_creation(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    println!("Sleeping in the test... ");
    std::thread::sleep(std::time::Duration::from_secs(5));
    println!("End sleeping");
    println!("Shutting down http service");
    context.server_handle.stop(true).await;
    println!("Http server closed");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_finish_request(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    println!("Sending finish request");
    let request_response = RollupResponse::Finish(true);
    match rollup_http_client::client::send_finish_request(&context.address, &request_response).await
    {
        Ok(request) => match request {
            RollupRequest::Inspect(_inspect_request) => {
                context.server_handle.stop(true).await;
                panic!("Got unexpected request")
            }
            RollupRequest::Advance(advance_request) => {
                println!("Got new advance request: {:?}", advance_request);
                assert_eq!(advance_request.payload.len(), 42);
                assert_eq!(
                    advance_request.metadata.msg_sender,
                    "0x1111111111111111111111111111111111111111"
                );
                assert_eq!(
                    std::str::from_utf8(&hex::decode(&advance_request.payload[2..]).unwrap())
                        .unwrap(),
                    "test advance request"
                );
            }
        },
        Err(err) => {
            context.server_handle.stop(true).await;
            return Err(Box::new(err));
        }
    }
    match rollup_http_client::client::send_finish_request(&context.address, &request_response).await
    {
        Ok(request) => match request {
            RollupRequest::Inspect(inspect_request) => {
                println!("Got new inspect request: {:?}", inspect_request);
                context.server_handle.stop(true).await;
                assert_eq!(inspect_request.payload.len(), 42);
                assert_eq!(
                    std::str::from_utf8(&hex::decode(&inspect_request.payload[2..]).unwrap())
                        .unwrap(),
                    "test inspect request"
                );
            }
            RollupRequest::Advance(_advance_request) => {
                context.server_handle.stop(true).await;
                panic!("Got unexpected request")
            }
        },
        Err(err) => {
            context.server_handle.stop(true).await;
            return Err(Box::new(err));
        }
    }
    context.server_handle.stop(true).await;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_ipfs_put_request(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    
    match rollup_http_client::client::ipfs_put_request(&context.address, "file content".to_string(), "filename").await {
        Ok(response) => {
            context.server_handle.stop(true).await;
            return Ok(())
        },
        Err(err) => {
            context.server_handle.stop(true).await;
            return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(),
        )))
    },

    }
}

#[rstest]
#[tokio::test]
async fn test_ipfs_get_request(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;

    match rollup_http_client::client::ipfs_get_request(&context.address, "filename").await {
        Ok(response) => {
                context.server_handle.stop(true).await;
                println!("{:?}", response);
                return Ok(())
            },
        Err(err) => {
            context.server_handle.stop(true).await;
            return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(),
        )))},

    }
}

#[rstest]
#[tokio::test]
async fn test_ipfs_has_request(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;

    match rollup_http_client::client::ipfs_has_request(&context.address, "QmNrN9SCRZSmVpcoAcKHAtvMCQmjPY8zDE66SZvQRp3zzB").await {
        Ok(response) => {
                context.server_handle.stop(true).await;
                return Ok(())
            },
        Err(err) => {
            context.server_handle.stop(true).await;
            return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(),
        )))},

    }
}

#[rstest]
#[tokio::test]
async fn test_write_voucher(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    println!("Writing voucher");
    let test_voucher_01 = Voucher {
        destination: "0x1111111111111111111111111111111111111111".to_string(),
        payload: "0x".to_string() + &hex::encode("voucher test payload 01"),
    };
    let test_voucher_02 = Voucher {
        destination: "0x2222222222222222222222222222222222222222".to_string(),
        payload: "0x".to_string() + &hex::encode("voucher test payload 02"),
    };
    rollup_http_client::client::send_voucher(&context.address, test_voucher_01).await;
    println!("Writing second voucher!");
    rollup_http_client::client::send_voucher(&context.address, test_voucher_02).await;
    context.server_handle.stop(true).await;

    //Read text file with results
    let voucher1 =
        std::fs::read_to_string("test_voucher_1.txt").expect("error reading voucher 1 file");
    assert_eq!(
        voucher1,
        "index: 1, payload_size: 23, payload: voucher test payload 01"
    );
    std::fs::remove_file("test_voucher_1.txt")?;

    let voucher2 =
        std::fs::read_to_string("test_voucher_2.txt").expect("error reading voucher 2 file");
    assert_eq!(
        voucher2,
        "index: 2, payload_size: 23, payload: voucher test payload 02"
    );
    std::fs::remove_file("test_voucher_2.txt")?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_reading(
) -> Result<(), Box<dyn std::error::Error>> {

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open("dev/file.test").unwrap();

    file.seek(SeekFrom::End(0)).unwrap();

    let file_length = file.stream_position().unwrap() as usize;

    let mut buffer: Vec<u8> = Vec::with_capacity(file_length);
    println!("file.stream position {:?}", file_length);

    file.seek(SeekFrom::Start(0)).unwrap();
    println!("file.stream position after seek to start {:?}", file.stream_position());

    for i in (0..file_length).step_by(4096) {
        let mut out_buf = Aligned([0; 4096 as usize]);
        file.read_exact(&mut out_buf.0).unwrap();
        buffer.extend_from_slice(&out_buf.0); 
    }

    assert_eq!(buffer.len() % 512, 0);
    let data_to_write = 0_u64.to_be_bytes();

    buffer.splice(8..8, data_to_write);
    file.set_len(0).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    for i in (0..buffer.len()).step_by(4096) {
        let chunk: [u8; 4096] = {
            let mut arr = [0; 4096];
            if i + 4096 > buffer.len() {
                let new_array = &buffer[i..buffer.len()];
                arr[..new_array.len()].copy_from_slice(new_array);
            }
            else {
                arr.copy_from_slice(&buffer[i..i + 4096]);
            }
            arr
        };
        let mut out_buf = Aligned(chunk);
        file.write(&mut out_buf.0).unwrap();
    }


    file.seek(SeekFrom::End(0)).unwrap();

    let file_length = file.stream_position().unwrap() as usize;

    println!("file_length {:?}", file_length);

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_write_notice(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    // Set the global log level
    println!("Writing notice");
    let test_notice = Notice {
        payload: "0x".to_string() + &hex::encode("notice test payload 01"),
    };
    rollup_http_client::client::send_notice(&context.address, test_notice).await;
    context.server_handle.stop(true).await;
    //Read text file with results
    let notice1 =
        std::fs::read_to_string("test_notice_1.txt").expect("error reading test notice file");
    assert_eq!(
        notice1,
        "index: 1, payload_size: 22, payload: notice test payload 01"
    );
    std::fs::remove_file("test_notice_1.txt")?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_write_report(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    // Set the global log level
    println!("Writing report");
    let test_report = Report {
        payload: "0x".to_string() + &hex::encode("report test payload 01"),
    };
    rollup_http_client::client::send_report(&context.address, test_report).await;
    context.server_handle.stop(true).await;
    //Read text file with results
    let report1 =
        std::fs::read_to_string("test_report_1.txt").expect("error reading test report file");
    assert_eq!(
        report1,
        "index: 1, payload_size: 22, payload: report test payload 01"
    );
    std::fs::remove_file("test_report_1.txt")?;

    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_exception_throw(
    context_future: impl Future<Output = Context>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = context_future.await;
    // Set the global log level
    println!("Throwing exception");
    let test_exception = Exception {
        payload: "0x".to_string() + &hex::encode("exception test payload 01"),
    };
    rollup_http_client::client::throw_exception(&context.address, test_exception).await;
    println!("Closing server after throw exception");
    context.server_handle.stop(true).await;
    println!("Server closed");
    //Read text file with results
    let exception =
        std::fs::read_to_string("test_exception_1.txt").expect("error reading test exception file");
    assert_eq!(
        exception,
        "index: 1, payload_size: 25, payload: exception test payload 01"
    );
    println!("Removing exception text file");
    std::fs::remove_file("test_exception_1.txt")?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn test_ipfs_interactions() -> Result<(), Box<dyn std::error::Error>> {
    let client = IpfsClient::default();

    let base_path = "/state/kv";
    let key = "test_key";
    let content = "Hello";
    let key_path = format!("{}/{}", base_path, key);

    client.files_mkdir(base_path, true).await.expect("Failed to create base directory");

    client.files_write(&key_path, true, true, Cursor::new(content.as_bytes())).await.expect("Failed to write initial value to IPFS");

    client.files_write(&key_path, true, true, Cursor::new(content.as_bytes())).await.expect("Failed to overwrite value in IPFS");

    let mut read_stream = client.files_read(&key_path);
    let mut bytes = BytesMut::new();
    while let Some(chunk) = read_stream.next().await {
        let chunk = chunk.expect("Failed to read chunk from stream");
        bytes.extend_from_slice(&chunk);
    }
    assert_eq!(std::str::from_utf8(&bytes)?, content, "The written and read content do not match");

    client.files_rm(&key_path, true).await.expect("Failed to remove file from IPFS");

    let mut read_stream = client.files_read(&key_path);
    let mut found_data = false;
    while let Some(chunk) = read_stream.next().await {
        if chunk.is_ok() {
            found_data = true;
            break;
        }
    }
    assert!(!found_data, "Expected no data for reading deleted value, but got some");

    Ok(())
}
