use anyhow::Result;
use dump::{db::adsbx::insert_aircraft, load_adsbx_json};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::{panic, process};
use structopt::StructOpt;
use tokio::runtime::Runtime;
use tokio_postgres::NoTls;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
}

fn main() -> Result<()> {
    // If any thread panics, exit the process.
    let orig_hook = panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);
        println!("Aborting");
        process::exit(1);
    }));

    let args = CliArgs::from_args();
    // Batch the paths into groups of 100.
    let bar = ProgressBar::new(args.paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} | {msg}"),
    );
    let path_groups = args.paths.chunks(100).collect::<Vec<_>>();
    let rt = Runtime::new()?;
    path_groups.par_iter().for_each(|paths| {
        let (client, connection) = rt
            .block_on(tokio_postgres::connect(
                "host=localhost user=adsbx password=adsbx dbname=adsbx",
                NoTls,
            ))
            .unwrap();
        rt.spawn(connection);
        paths.iter().for_each(|path| {
            bar.inc(1);
            let adsbx_data = load_adsbx_json(path).unwrap();
            let now = adsbx_data.now;
            let aircraft = adsbx_data.aircraft;
            for ac in aircraft {
                rt.block_on(insert_aircraft(&client, &now, &ac)).unwrap();
            }
        });
    });
    bar.finish();
    Ok(())
}
