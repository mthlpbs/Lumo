//! Brave's adblock-rust engine. Built only for opted-in embedded web views;
//! there is no handwritten rule parser and no bundled filter-list body.
use adblock::{lists::{FilterSet, ParseOptions}, request::Request, resources::Resource};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;

pub struct Engine(adblock::Engine);
pub const DOCUMENT:u32=1;
pub const SUBDOCUMENT:u32=2;
pub const SCRIPT:u32=4;
pub const IMAGE:u32=8;
pub const STYLESHEET:u32=16;
pub const FONT:u32=32;
pub const MEDIA:u32=64;
pub const XHR:u32=128;
pub const PING:u32=256;
pub const OTHER:u32=512;

#[derive(Default,Debug)]
pub struct Decision {
    pub blocked:bool,
    pub redirect:Option<(String,Vec<u8>)>,
    pub rewritten_url:Option<String>,
}
#[derive(Default,Serialize)]
pub struct Cosmetics {
    pub selectors:Vec<String>,
    pub actions:Vec<serde_json::Value>,
    #[serde(skip)]
    pub scriptlets:String,
    pub generic:bool,
}
fn kind(kind:u32)->&'static str {
    match kind {DOCUMENT=>"document",SUBDOCUMENT=>"subdocument",SCRIPT=>"script",IMAGE=>"image",STYLESHEET=>"stylesheet",FONT=>"font",MEDIA=>"media",XHR=>"xmlhttprequest",PING=>"ping",_=>"other"}
}
impl Engine {
    pub fn from_lists(texts:impl IntoIterator<Item=String>, resources:&str)->Result<Self,String> {
        let mut set=FilterSet::new(false);
        for text in texts {set.add_filter_list(text,ParseOptions::default());}
        let mut engine=adblock::Engine::new_with_filter_set(set);
        let resources:Vec<Resource>=serde_json::from_str(resources).map_err(|e|format!("Invalid protection resources: {e}"))?;
        engine.use_resources(resources);
        Ok(Self(engine))
    }
    pub fn check(&self,url:&str,source:&str,resource:u32,method:&str)->Decision {
        let Ok(request)=Request::new(url,source,kind(resource),method)else{return Decision::default();};
        let result=self.0.check_network_request(&request);
        let blocked=result.should_block();
        let redirect=if blocked {result.redirect.and_then(|s|decode_redirect(&s))}else{None};
        Decision{blocked,redirect,rewritten_url:if blocked{None}else{result.rewritten_url}}
    }
    pub fn blocked(&self,url:&str,source:&str,resource:u32)->bool {self.check(url,source,resource,"GET").blocked}
    pub fn cosmetics(&self,url:&str,classes:&[String],ids:&[String],initial:bool)->Cosmetics {
        let page=self.0.url_cosmetic_resources(url);
        let mut selectors=if initial{page.hide_selectors.into_iter().collect()}else{Vec::new()};
        if !page.generichide {selectors.extend(self.0.hidden_class_id_selectors(classes,ids,&page.exceptions));}
        let actions=if initial{page.procedural_actions.into_iter().filter_map(|s|serde_json::from_str(&s).ok()).collect()}else{Vec::new()};
        Cosmetics{selectors,actions,scriptlets:if initial{page.injected_script}else{String::new()},generic:!page.generichide}
    }
}
fn decode_redirect(data:&str)->Option<(String,Vec<u8>)> {
    let (header,body)=data.strip_prefix("data:")?.split_once(',')?;
    let mime=header.split(';').next()?.to_string();
    if !header.ends_with(";base64"){return None;}
    Some((mime,STANDARD.decode(body).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn engine(rules:&str)->Engine{Engine::from_lists([rules.into()],"[]").unwrap()}
    #[test]
    fn brave_handles_regex_exceptions_resource_types_and_real_site_domains(){
        let e=engine("/ads[0-9]+\\.js/$script\n@@||ads.example.test/allowed\n||ads.example.test^\n||cdn.example.co.uk^$third-party,image\n||tracker.test^$domain=news.test|~safe.news.test");
        assert!(e.blocked("https://x.test/ads123.js","https://news.test/",SCRIPT));
        assert!(!e.blocked("https://x.test/ads123.js","https://news.test/",IMAGE));
        assert!(!e.blocked("https://ads.example.test/allowed","https://news.test/",SCRIPT));
        assert!(!e.blocked("https://cdn.example.co.uk/ad","https://www.example.co.uk/",IMAGE));
        assert!(e.blocked("https://cdn.example.co.uk/ad","https://unrelated.co.uk/",IMAGE));
        assert!(!e.blocked("https://tracker.test/","https://safe.news.test/",SCRIPT));
        assert!(engine("||ads.test^$script").blocked("https://ads.test/blocked.js","about:blank",SCRIPT));
    }
    #[test]
    fn brave_returns_dynamic_generic_css_and_procedural_actions(){
        let e=engine("##.advert\nexample.test##.sponsored\nexample.test##.card:has-text(Sponsored)\nexample.test#@#.allowed");
        let c=e.cosmetics("https://example.test/",&["advert".into()],&[],true);
        assert!(c.selectors.iter().any(|s|s==".advert"));assert!(c.selectors.iter().any(|s|s==".sponsored"));
        assert!(!c.actions.is_empty());
    }
    #[test]
    fn brave_provides_redirect_resources_and_parameter_removal(){
        let resources=r#"[{"name":"noop.js","aliases":["noopjs"],"kind":{"mime":"application/javascript"},"content":"Ly8gZW1wdHk="}]"#;
        let e=Engine::from_lists(["||ads.test^$script,redirect=noopjs\n||news.test^$removeparam=tracking".into()],resources).unwrap();
        let d=e.check("https://ads.test/ad.js","https://news.test/",SCRIPT,"GET");
        assert!(d.blocked);assert_eq!(d.redirect.unwrap().1,b"// empty");
        assert_eq!(e.check("https://news.test/?tracking=1&keep=2","https://news.test/",DOCUMENT,"GET").rewritten_url.as_deref(),Some("https://news.test/?keep=2"));
    }
    #[test]
    fn brave_resolves_scriptlet_templates_from_verified_resources(){
        let resources=r#"[{"name":"fixture.js","kind":"template","content":"d2luZG93Ll9fZml4dHVyZT17ezF9fTs="}]"#;
        let e=Engine::from_lists(["example.test##+js(fixture, true)".into()],resources).unwrap();
        let c=e.cosmetics("https://example.test/",&[],&[],true);
        assert!(c.scriptlets.contains("window.__fixture=true"));
    }
}
