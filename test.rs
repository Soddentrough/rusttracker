use egui_file_dialog::{FileDialog, DialogState};
fn main() {
    let mut d = FileDialog::new();
    d.pick_file();
    println!("{:?}", d.state());
}
