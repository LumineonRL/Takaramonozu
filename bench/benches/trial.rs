use divan::Bencher;

fn names() -> impl Iterator<Item = &'static str> {
    // this will get all of the names from each file in `specs`.
    todo!() as std::iter::Empty<&'static str>
}

fn main() {
    divan::main();
}

#[divan::bench(args = names())]
fn trial(_bencher: Bencher, _name: &str) {
    todo!()
}
