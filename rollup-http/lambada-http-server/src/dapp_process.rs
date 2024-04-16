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

use tokio::process::Command;

use crate::rollup::Exception;

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
    let exception = Exception {
        payload: String::from("0x") + &hex::encode(message),
    };
    let client = hyper::Client::new();
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .uri("http://127.0.0.1:5004/exception")
        .body(hyper::Body::from(
            serde_json::to_string(&exception).unwrap(),
        ))
        .expect("exception request");
    match client.request(req).await {
        Ok(_) => {
            log::debug!("exception successfully thrown {:#?}", exception);
        }
        Err(e) => {
            log::error!("unable to throw exception, error details: '{}'", e);
        }
    };
}
