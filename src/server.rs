use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use std::hash::{Hasher, Hash};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use hello_world::greeter_server::{Greeter, GreeterServer};
use hello_world::{Feature, HelloReply, HelloRequest, Point, Rectangle, RouteNote, RouteSummary};

pub mod hello_world {
    tonic::include_proto!("helloworld"); // The string specified here must match the proto package name
}

#[derive(Debug, Default)]
pub struct MyGreeter {
    features: Arc<Vec<Feature>>,
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
        let features = self.features.clone();

        tokio::spawn(async move {
            for feature in &features[..] {
                if in_range(feature.location.as_ref().unwrap(), request.get_ref()) {
                    info!("  => send {:?}", feature);
                    tx.send(Ok(feature.clone())).await.unwrap();
                }
            }

            info!(" /// done sending");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
    async fn get_feature(&self, request: Request<Point>) -> Result<Response<Feature>, Status> {
        info!("GetFeature = {:?}", request);

        for feature in &self.features[..] {
            if feature.location.as_ref() == Some(request.get_ref()) {
                return Ok(Response::new(feature.clone()));
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

            // Find features
            for feature in self.features.iter() {
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

fn in_range(point: &Point, rect: &Rectangle) -> bool {
    use std::cmp;

    let lo = rect.lo.as_ref().unwrap();
    let hi = rect.hi.as_ref().unwrap();

    let left = cmp::min(lo.longitude, hi.longitude);
    let right = cmp::max(lo.longitude, hi.longitude);
    let top = cmp::max(lo.latitude, hi.latitude);
    let bottom = cmp::min(lo.latitude, hi.latitude);

    point.longitude >= left
        && point.longitude <= right
        && point.latitude >= bottom
        && point.latitude <= top
}

/// Calculates the distance between two points using the "haversine" formula.
/// This code was taken from http://www.movable-type.co.uk/scripts/latlong.html.
fn calc_distance(p1: &Point, p2: &Point) -> i32 {
    const CORD_FACTOR: f64 = 1e7;
    const R: f64 = 6_371_000.0; // meters

    let lat1 = p1.latitude as f64 / CORD_FACTOR;
    let lat2 = p2.latitude as f64 / CORD_FACTOR;
    let lng1 = p1.longitude as f64 / CORD_FACTOR;
    let lng2 = p2.longitude as f64 / CORD_FACTOR;

    let lat_rad1 = lat1.to_radians();
    let lat_rad2 = lat2.to_radians();

    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lng = (lng2 - lng1).to_radians();

    let a = (delta_lat / 2f64).sin() * (delta_lat / 2f64).sin()
        + (lat_rad1).cos() * (lat_rad2).cos() * (delta_lng / 2f64).sin() * (delta_lng / 2f64).sin();

    let c = 2f64 * a.sqrt().atan2((1f64 - a).sqrt());

    (R * c) as i32
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter::default();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(tracing_subscriber::fmt::layer())
        .init();
    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}
