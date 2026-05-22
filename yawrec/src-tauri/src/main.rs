// Empêche l'apparition d'une console en release sur Windows.
// NE PAS SUPPRIMER : sans cette ligne, lancer le .exe ouvre cmd.exe en parallèle.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    yawrec_lib::run();
}
