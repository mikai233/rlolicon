use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::{App, HttpResponse, HttpServer, Responder};
use actix_web::get;
use log::{error, info};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Serialize, Deserialize, Debug)]
struct LoliconResp {
    error: String,
    data: Vec<LoliconSetu>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LoliconSetu {
    pid: u64,
    p: u64,
    uid: u64,
    title: String,
    author: String,
    r18: bool,
    width: u16,
    height: u16,
    tags: Vec<String>,
    ext: String,
    upload_date: u64,
    urls: Url,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Url {
    original: Option<String>,
    regular: Option<String>,
    small: Option<String>,
    mini: Option<String>,
}

#[get("/")]
async fn setu(data: actix_web::web::Data<Arc<Mutex<Receiver<Vec<u8>>>>>) -> impl Responder {
    match data.lock().unwrap().try_recv() {
        Ok(data) => {
            HttpResponse::Ok().body(data)
        }
        Err(err) => {
            error!("{}",err);
            HttpResponse::InternalServerError().body("image not ready")
        }
    }
}

const API: &str = "https://api.lolicon.app/setu/v2";
const BYTES_IMAGE_NUM: usize = 50;
const IMAGE_INFO_NUM: usize = 100;
const WORKER_NUM: usize = 3;
const WORKER_CACHE: usize = 10;

#[actix_web::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();
    let resp_rx = download_api_info();
    let (image_tx, image_rx) = tokio::sync::mpsc::channel(BYTES_IMAGE_NUM);
    let image_rx = Arc::new(Mutex::new(image_rx));
    download_image(resp_rx, image_tx);
    let _ = HttpServer::new(move || {
        App::new()
            .service(setu)
            .app_data(actix_web::web::Data::new(image_rx.clone()))
    })
        .bind("0.0.0.0:8080")?
        .run()
        .await;
    Ok(())
}

fn download_api_info() -> Receiver<LoliconResp> {
    let (tx, rx) = tokio::sync::mpsc::channel::<LoliconResp>(IMAGE_INFO_NUM);
    tokio::spawn(async move {
        loop {
            match req_lolicon_info().await {
                Ok(resp) => {
                    match tx.send(resp).await {
                        Ok(_) => {}
                        Err(err) => {
                            error!("{}",err)
                        }
                    };
                }
                Err(err) => {
                    error!("{}",err)
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    rx
}

async fn req_lolicon_info() -> Result<LoliconResp, reqwest::Error> {
    let mut url = reqwest::Url::parse(API).unwrap();
    url.set_query(Some("r18=2"));
    url.set_query(Some("num=10"));
    let resp = reqwest::get(url)
        .await?
        .json::<LoliconResp>()
        .await?;
    Ok(resp)
}

fn download_image(mut resp_rx: Receiver<LoliconResp>, image_tx: Sender<Vec<u8>>) {
    tokio::spawn(async move {
        let image_tx = Arc::new(image_tx);
        let mut current_worker: usize = 0;
        let mut workers = Vec::with_capacity(WORKER_NUM);
        for _ in 0..WORKER_NUM {
            let (w_tx, mut w_rx) = tokio::sync::mpsc::channel::<String>(WORKER_CACHE);
            let image_tx = image_tx.clone();
            tokio::spawn(async move {
                while let Some(url) = w_rx.recv().await {
                    match reqwest::get(url).await {
                        Ok(resp) => {
                            match resp.bytes().await {
                                Ok(bytes) => {
                                    let _ = image_tx.as_ref().send(bytes.to_vec()).await;
                                }
                                Err(err) => {
                                    error!("{}",err);
                                }
                            }
                        }
                        Err(err) => {
                            error!("{}",err);
                        }
                    }
                }
            });
            workers.push(w_tx);
        }
        while let Some(resp) = resp_rx.recv().await {
            for data in resp.data {
                info!("{}",serde_json::to_string(&data).unwrap());
                if let Some(url) = data.urls.original {
                    let worker = &workers[current_worker];
                    let _ = worker.send(url).await;
                    current_worker = (current_worker + 1) % WORKER_NUM;
                }
            }
        }
    });
}