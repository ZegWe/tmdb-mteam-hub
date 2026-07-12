#[tokio::main]
async fn main() {
    if let Err(error) = tmdb_mteam_server::run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
