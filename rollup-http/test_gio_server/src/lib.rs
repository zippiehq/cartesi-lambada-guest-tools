use hyper::service::{make_service_fn, service_fn};
use hyper::Method;
use hyper::{Body, Request, Response, Server};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use tokio::sync::oneshot;
async fn handle_request(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::POST, "/gio") => {
            let result = GIOResponse {
                response_code: 0,
                response: "0x".to_string(),
            };
            Ok(Response::new(Body::from(json!(result).to_string())))
        }
        _ => {
            let not_found = Response::builder()
                .status(404)
                .body(Body::from("404 Not Found"))
                .unwrap();
            Ok(not_found)
        }
    }
}

pub async fn start_server(tx: oneshot::Sender<()>) {
    let make_svc =
        make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle_request)) });

    let addr = ([127, 0, 0, 1], 5004).into();
    let server = Server::bind(&addr).serve(make_svc);

    let _ = tx.send(());

    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GIOResponse {
    pub response_code: u16,
    pub response: String,
}
