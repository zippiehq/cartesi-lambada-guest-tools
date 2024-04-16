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

use std::sync::Arc;

use crate::config::Config;
use crate::rollup::{GIORequest, GIOResponse};
use actix_web::web;
use actix_web::web::{Bytes, BytesMut};
use actix_web::{middleware::Logger, App, HttpResponse, HttpServer};
use cid::Cid;
use futures::StreamExt;
use ipfs_api_backend_hyper::{IpfsApi, IpfsClient, TryFromUri};
use std::io::Cursor;
use tokio::sync::Notify;
const CURRENT_STATE_CID: u16 = 0x20;
const SET_STATE_CID: u16 = 0x21;

/// Create new instance of http server
pub fn create_server(config: &Config) -> std::io::Result<actix_server::Server> {
    let server = HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .service(open_state)
            .service(commit_state)
            .service(delete_state)
            .service(set_state)
            .service(get_state)
    })
    .bind((config.http_address.as_str(), config.http_port))
    .map(|t| t)?
    .run();
    Ok(server)
}

/// Create and run new instance of http server
pub async fn run(config: &Config, server_ready: Arc<Notify>) -> std::io::Result<()> {
    log::info!("starting http dispatcher http service!");
    let server = create_server(config)?;
    server_ready.notify_one();
    server.await
}

// Deletes state with a particular key
#[actix_web::delete("/delete_state/{key}")]
async fn delete_state(key: web::Path<String>) -> HttpResponse {
    let client = IpfsClient::from_str("http://127.0.0.1:5001").unwrap();
    let key_path = format!("/state/kv/{}", key.into_inner());
    match client.files_rm(&key_path, true).await {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

// Sets state with a particular key
#[actix_web::post("/set_state/{key}")]
async fn set_state(key: web::Path<String>, body: Bytes) -> HttpResponse {
    let client = IpfsClient::from_str("http://127.0.0.1:5001").unwrap();
    let base_path = "/state/kv";
    let _ = client.files_mkdir(base_path, true).await;
    let key_path = format!("{}/{}", base_path, key.into_inner());

    let reader = Cursor::new(body);

    match client.files_write(&key_path, true, true, reader).await {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}

// Receives state with a particular key
#[actix_web::get("/get_state/{key}")]
async fn get_state(key: web::Path<String>) -> HttpResponse {
    let client = IpfsClient::from_str("http://127.0.0.1:5001").unwrap();
    let key_path = format!("/state/kv/{}", key.into_inner());

    let stream = client.files_read(&key_path);
    let result = stream
        .fold(BytesMut::new(), |mut acc, item| async move {
            match item {
                Ok(chunk) => {
                    acc.extend_from_slice(&chunk);
                    acc
                }
                Err(_) => acc,
            }
        })
        .await;

    HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(result.freeze())
}

// Receives state with a particular key
#[actix_web::get("/open_state")]
async fn open_state() -> HttpResponse {
    let gio_request = GIORequest {
        domain: CURRENT_STATE_CID,
        payload: String::new(),
    };

    let client = hyper::Client::new();

    //Request for getting state_cid from rollup_http_server qio request
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .uri("http://127.0.0.1:5004/gio")
        .body(hyper::Body::from(
            serde_json::to_string(&gio_request).unwrap(),
        ))
        .expect("gio request");
    match client.request(req).await {
        Ok(gio_response) => {
            let gio_response = serde_json::from_slice::<GIOResponse>(
                &hyper::body::to_bytes(gio_response)
                    .await
                    .expect("error get response from rollup_http_server qio request")
                    .to_vec(),
            )
            .unwrap();

            let endpoint = "http://127.0.0.1:5001".to_string();
            let client = IpfsClient::from_str(&endpoint).unwrap();
            let cid = Cid::try_from(gio_response.response.clone()).unwrap();

            // Updates new state using cid received from rollup_http_server qio request
            client
                .files_cp(&("/ipfs/".to_string() + &cid.to_string()), "/state-new")
                .await
                .unwrap();
            client.files_rm("/state-new/previous", true).await.unwrap();
            client
                .files_cp(
                    &("/ipfs/".to_string() + &cid.to_string()),
                    "/state-new/previous",
                )
                .await
                .unwrap();
            client.files_rm("/state", true).await.unwrap();
            client.files_mv("/state-new", "/state").await.unwrap();

            HttpResponse::Ok()
                .append_header((hyper::header::CONTENT_TYPE, "application/octet-stream"))
                .body(gio_response.response)
        }
        Err(e) => {
            log::error!("failed to handle open_state request: {}", e);
            HttpResponse::BadRequest().body(format!("Failed to handle open_state request: {}", e))
        }
    }
}

#[actix_web::get("/commit_state")]
async fn commit_state() -> HttpResponse {
    let endpoint = "http://127.0.0.1:5001".to_string();
    let client = IpfsClient::from_str(&endpoint).unwrap();
    let cid = client.files_stat("/state").await.unwrap().hash;
    let cid = Cid::try_from(cid).unwrap();
    let cid_bytes = cid.to_bytes();

    let gio_request = GIORequest {
        domain: SET_STATE_CID,
        payload: hex::encode(cid_bytes),
    };
    let client = hyper::Client::new();

    // rollup_http_server gio request with cid received from /state
    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .uri("http://127.0.0.1:5004/gio")
        .body(hyper::Body::from(
            serde_json::to_string(&gio_request).unwrap(),
        ))
        .expect("gio request");

    match client.request(req).await {
        Ok(gio_response) => {
            let gio_response = serde_json::from_slice::<GIOResponse>(
                &hyper::body::to_bytes(gio_response)
                    .await
                    .expect("error get response from rollup_http_server qio request")
                    .to_vec(),
            )
            .unwrap();

            HttpResponse::Ok()
                .append_header((hyper::header::CONTENT_TYPE, "application/octet-stream"))
                .body(gio_response.response)
        }
        Err(e) => {
            log::error!("failed to handle commit_state request: {}", e);
            HttpResponse::BadRequest().body(format!("Failed to handle commit_state request: {}", e))
        }
    }
}
