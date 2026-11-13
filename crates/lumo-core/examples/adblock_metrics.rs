use lumo_core::adblocking as engine;
fn load_engine(lists:Vec<String>,resources:String)->engine::Engine {
    engine::Engine::from_lists(lists,&resources).unwrap()
}
include!("../../../tools/adblock_metrics_common.rs");
