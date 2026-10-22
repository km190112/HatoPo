// exe にアイコンと版情報を埋め込む (エクスプローラーで鳩の絵が出る)。
fn main() {
    println!("cargo:rerun-if-changed=../../assets/hatopo.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/hatopo.ico")
        .set("ProductName", "HatoPo")
        .set("FileDescription", "HatoPo — デスクトップの小さな伝書鳩")
        .set("OriginalFilename", "HatoPo.exe");
    if let Err(e) = res.compile() {
        panic!("リソースを埋め込めません: {e}");
    }
}
