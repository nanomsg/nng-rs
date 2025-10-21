fn main() {
    println!("cargo::rustc-check-cfg=cfg(nng_110)");

    let version = std::env::var("DEP_NNG_VERSION").expect("nng-sys always exposes nng version");
    let mut components = version.split('.');
    let _major: u64 = components
        .next()
        .expect("nng versions always have three components")
        .parse()
        .expect("nng version components are integers");
    let minor: u64 = components
        .next()
        .expect("nng versions always have three components")
        .parse()
        .expect("nng version components are integers");
    let _patch: u64 = components
        .next()
        .expect("nng versions always have three components")
        .parse()
        .expect("nng version components are integers");
    if minor >= 10 {
        println!("cargo::rustc-cfg=nng_110");
    }
}
