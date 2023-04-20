use anyhow::Result;
use structopt::StructOpt;
use tracon::{
    for_each_adsbx_json,
    interception::{url, State},
};

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
    #[structopt(long, help = "Skip JSON decoding errors")]
    pub skip_json_errors: bool,
}

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    eprintln!("Processing {} files", args.paths.len());
    let mut state = State::default();
    for_each_adsbx_json(&args.paths, args.skip_json_errors, |response, bar| {
        tracon::interception::process_adsbx_response(&mut state, response, bar)
    })
    .unwrap();
    eprintln!(
        "Indexed {} aircraft, processed {} aircraft, found {} interceptions",
        state.num_ac_indexed,
        state.num_ac_processed,
        state.interceptions.len()
    );
    for interception in state.interceptions {
        println!("{} {} intercepted {} at {} with {:.0} ft lateral separation, {} ft vertical separation",
        url(&interception.interceptor, &interception.target, interception.time),
        interception.interceptor.hex,
             interception.target.hex,
             interception.time,
             interception.lateral_separation_ft.round(),
             interception.vertical_separation_ft,
        );
    }
    Ok(())
}
