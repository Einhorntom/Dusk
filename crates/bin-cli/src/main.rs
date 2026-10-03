fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match dispcontrol_cli::run(&args) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            if error.json_output {
                println!("{}", error.message);
            } else {
                eprintln!("{}", error.message);
            }
            std::process::exit(error.exit_code);
        }
    }
}
