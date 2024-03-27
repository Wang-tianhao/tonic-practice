use anyhow::Result;
use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{Feature, HelloReply, HelloRequest, Point, Rectangle, RouteNote, RouteSummary};
// use helloworld_tonic::prisma::location::{self, latitude, longitude};
use helloworld_tonic::{config::app_config::AppConfig, prisma::PrismaClient};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{transport::Server, Request, Response, Status};
// use tower::{Layer, Service};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use middleware::{intercept, MyMiddlewareLayer};

use utils::{calc_distance, in_range};

mod data;
mod domain;
mod middleware;
mod utils;

pub mod hello_world {
    tonic::include_proto!("helloworld"); // The string specified here must match the proto package name
}

#[derive(Debug)]
pub struct MyGreeter {
    // features: Arc<Vec<Feature>>,
    prisma: Arc<PrismaClient>,
    features: Arc<Mutex<Vec<Feature>>>,
}

impl Hash for Point {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.latitude.hash(state);
        self.longitude.hash(state);
    }
}
impl Eq for Point {}

impl MyGreeter {
    async fn get_features(&self) {
        let features = self
            .prisma
            .location()
            .find_many(vec![])
            .exec()
            .await
            .unwrap();
        let return_feature: Vec<Feature> = features
            .into_iter()
            .map(|feature| Feature {
                name: feature.name,
                location: Some(Point {
                    longitude: feature.longitude,
                    latitude: feature.latitude,
                }),
            })
            .collect();
        *self.features.lock().await = return_feature;
    }
}
#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>, // Accept request of type HelloRequest
    ) -> Result<Response<HelloReply>, Status> {
        // Return an instance of type HelloReply
        info!("Got a request: {:?}", request);

        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name), // We must use .into_inner() as the fields of gRPC requests and responses are private
        };

        Ok(Response::new(reply)) // Send back our formatted greeting
    }
    type ListFeaturesStream = ReceiverStream<Result<Feature, Status>>;
    async fn list_features(
        &self,
        request: Request<Rectangle>,
    ) -> Result<Response<Self::ListFeaturesStream>, Status> {
        info!("ListFeatures = {:?}", request);

        let (tx, rx) = mpsc::channel(4);
        // let features = self.features.clone();
        let mut features = self.features.lock().await.to_vec();
        if features.len() == 0 {
            self.get_features().await;
            features = self.features.lock().await.to_vec();
        }
        tokio::spawn(async move {
            for feature in features.into_iter() {
                if in_range(&feature.location.clone().unwrap(), request.get_ref()) {
                    info!("  => send {:?}", feature);
                    let _ = tx.send(Ok(feature)).await;
                }
            }

            info!(" /// done sending");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
    async fn get_feature(&self, request: Request<Point>) -> Result<Response<Feature>, Status> {
        info!("GetFeature = {:?}", request);
        let mut features = self.features.lock().await.to_vec();
        if features.len() == 0 {
            self.get_features().await;
            features = self.features.lock().await.to_vec();
        }
        for feature in features.into_iter() {
            if &feature.location.clone().unwrap() == request.get_ref() {
                return Ok(Response::new(feature));
            }
        }

        Ok(Response::new(Feature::default()))
    }
    async fn record_route(
        &self,
        request: Request<tonic::Streaming<Point>>,
    ) -> Result<Response<RouteSummary>, Status> {
        info!("RecordRoute");

        let mut stream = request.into_inner();

        let mut summary = RouteSummary::default();
        let mut last_point = None;
        let now = Instant::now();

        while let Some(point) = stream.next().await {
            let point = point?;

            info!("  ==> Point = {:?}", point);

            // Increment the point count
            summary.point_count += 1;
            let mut features = self.features.lock().await.to_vec();
            if features.len() == 0 {
                self.get_features().await;
                features = self.features.lock().await.to_vec();
            }
            // Find features
            for feature in features.iter() {
                if feature.location.as_ref() == Some(&point) {
                    summary.feature_count += 1;
                }
            }

            // Calculate the distance
            if let Some(ref last_point) = last_point {
                summary.distance += calc_distance(last_point, &point);
            }

            last_point = Some(point);
        }

        summary.elapsed_time = now.elapsed().as_secs() as i32;

        Ok(Response::new(summary))
    }

    type RouteChatStream = Pin<Box<dyn Stream<Item = Result<RouteNote, Status>> + Send + 'static>>;

    async fn route_chat(
        &self,
        request: Request<tonic::Streaming<RouteNote>>,
    ) -> Result<Response<Self::RouteChatStream>, Status> {
        info!("RouteChat");

        let mut notes = HashMap::new();
        let mut stream = request.into_inner();

        let output = async_stream::try_stream! {
            while let Some(note) = stream.next().await {
                let note = note?;

                let location = note.location.clone().unwrap();

                let location_notes = notes.entry(location).or_insert(vec![]);
                location_notes.push(note);

                for note in location_notes {
                    yield note.clone();
                }
            }
        };

        Ok(Response::new(Box::pin(output) as Self::RouteChatStream))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config: AppConfig = AppConfig::init();
    //*  App Context if needed
    // let app_context = AppContext {
    //     config: Arc::new(config.clone()),
    // };
    let addr = "[::1]:50051".parse()?;
    let prisma_client = Arc::new(PrismaClient::_builder().build().await?);
    let greeter = MyGreeter {
        // features: Arc::new(data::load()),
        features: Arc::new(Mutex::new(vec![])),
        prisma: prisma_client,
    };
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(&config.log_level))
        .with(tracing_subscriber::fmt::layer())
        .init();
    let layer = tower::ServiceBuilder::new()
        .timeout(Duration::from_secs(30))
        .layer(MyMiddlewareLayer::default())
        .layer(tonic::service::interceptor(intercept))
        .into_inner();
    Server::builder()
        .layer(layer)
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}
