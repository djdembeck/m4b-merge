use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "m4b-merge")]
#[command(author = "djdembeck")]
#[command(version = "0.1.0")]
#[command(about = "A CLI tool which outputs consistently sorted, tagged, single m4b files", long_about = None)]
struct Args {
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    println!("Hello, world!");
    if args.verbose {
        println!("Verbose mode enabled");
    }
}
