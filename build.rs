use vergen::{ConstantsFlags, generate_cargo_keys};

fn main() {
    let flags = ConstantsFlags::SHA;
    generate_cargo_keys(flags).expect("Unable to generate the cargo keys!");
}
