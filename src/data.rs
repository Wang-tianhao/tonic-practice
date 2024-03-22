use serde::Deserialize;
use std::fs::File;

use helloworld_tonic::prisma::PrismaClient;
#[derive(Debug, Deserialize)]
struct Feature {
    location: Location,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Location {
    latitude: i32,
    longitude: i32,
}

#[allow(dead_code)]
pub fn load() -> Vec<crate::hello_world::Feature> {
    let data_dir = std::path::PathBuf::from_iter([std::env!("CARGO_MANIFEST_DIR"), "data"]);
    let file = File::open(data_dir.join("route_guide_db.json")).expect("failed to open data file");

    let decoded: Vec<Feature> =
        serde_json::from_reader(&file).expect("failed to deserialize features");

    decoded
        .into_iter()
        .map(|feature| crate::hello_world::Feature {
            name: feature.name,
            location: Some(crate::hello_world::Point {
                longitude: feature.location.longitude,
                latitude: feature.location.latitude,
            }),
        })
        .collect()
}

#[allow(dead_code)]
// #[tokio::main]
pub async fn seed() {
    let data_dir = std::path::PathBuf::from_iter([std::env!("CARGO_MANIFEST_DIR"), "data"]);
    let file = File::open(data_dir.join("route_guide_db.json")).expect("failed to open data file");

    let prisma_client = PrismaClient::_builder().build().await.unwrap();
    let decoded: Vec<Feature> =
        serde_json::from_reader(&file).expect("failed to deserialize features");
    for rec in decoded {
        prisma_client
            .location()
            .create(
                rec.name,
                rec.location.latitude,
                rec.location.longitude,
                vec![],
            )
            .exec()
            .await
            .unwrap();
    }
}
