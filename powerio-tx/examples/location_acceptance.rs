use std::path::PathBuf;
fn visit(path: PathBuf) {
    if path.is_dir() {
        for item in std::fs::read_dir(path).unwrap() {
            visit(item.unwrap().path());
        }
        return;
    }
    if !path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("pwb"))
    {
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    match powerio_tx::format::powerworld::__parse_pwb(&bytes, Some("acceptance")) {
        Ok(net) => {
            let located = net.buses().iter().filter(|b| b.location.is_some()).count();
            println!(
                "{} {}/{} {}",
                path.display(),
                located,
                net.buses().len(),
                net.buses()
                    .first()
                    .and_then(|b| b.location)
                    .map(|l| format!("{},{}", l.x, l.y))
                    .unwrap_or_default()
            );
            assert_eq!(located, net.buses().len());
        }
        Err(e) => panic!("{}: {e}", path.display()),
    }
}
fn main() {
    visit(std::env::args().nth(1).unwrap().into());
}
