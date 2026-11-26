use crate::accounts::{Account, owner};
use crate::api::{Api, report, rules, settle};
use serde_json::{Map, Value, json};

const MANGA: u8 = 0;
const BOOK: u8 = 2;
const ARCHIVE: u8 = 1;
const EPUB: u8 = 2;
const PDF: u8 = 3;
const HARDCOVER: u8 = 2;
const MANGABAKA: u8 = 3;
const ROLES: [&str; 3] = ["Login", "Download", "Bookmark"];
const NO_AGE_RATING: i64 = -1;
const LARGEST_COVERS: u8 = 4;

#[derive(Debug, PartialEq)]
struct Shelf {
    name: String,
    kind: u8,
    provider: u8,
    folder: String,
    groups: Vec<u8>,
}

fn public() -> Vec<Shelf> {
    vec![
        Shelf {
            name: "Livres".into(),
            kind: BOOK,
            provider: HARDCOVER,
            folder: "/data/media/books".into(),
            groups: vec![EPUB, PDF],
        },
        Shelf {
            name: "Mangas".into(),
            kind: MANGA,
            provider: MANGABAKA,
            folder: "/data/media/manga".into(),
            groups: vec![ARCHIVE],
        },
    ]
}

fn private(account: &Account) -> Shelf {
    Shelf {
        name: account.name.clone(),
        kind: BOOK,
        provider: HARDCOVER,
        folder: account.books(),
        groups: vec![ARCHIVE, EPUB, PDF],
    }
}

fn folders(library: &Value) -> impl Iterator<Item = &str> {
    library["folders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|folder| folder.trim_end_matches('/'))
}

fn held(libraries: &Value, folder: &str) -> bool {
    libraries
        .as_array()
        .into_iter()
        .flatten()
        .any(|library| folders(library).any(|path| path == folder))
}

fn name_taken(libraries: &Value, name: &str) -> bool {
    libraries
        .as_array()
        .into_iter()
        .flatten()
        .any(|library| library["name"].as_str() == Some(name))
}

fn create(api: &impl Api, libraries: &Value, shelf: &Shelf, id: &str) -> Result<(), String> {
    let name = if name_taken(libraries, &shelf.name) {
        format!("{} ({id})", shelf.name)
    } else {
        shelf.name.clone()
    };
    api.send(
        "POST",
        "/library/create",
        &json!({
            "id": 0,
            "name": name,
            "type": shelf.kind,
            "folders": [shelf.folder],
            "folderWatching": true,
            "includeInDashboard": true,
            "includeInSearch": true,
            "manageCollections": true,
            "manageReadingLists": true,
            "allowScrobbling": false,
            "allowMetadataMatching": false,
            "enableMetadata": true,
            "removePrefixForSortName": false,
            "inheritWebLinksFromFirstChapter": false,
            "defaultLanguage": "",
            "fileGroupTypes": shelf.groups,
            "excludePatterns": [],
            "metadataProvider": shelf.provider,
        }),
    )?;
    println!("Kavita libraries: added {name} on {}", shelf.folder);
    Ok(())
}

fn shelve(api: &impl Api, accounts: &[Account]) -> Result<usize, String> {
    let libraries = api.get("/library/libraries")?;
    let mut created = 0;
    for shelf in public() {
        if !held(&libraries, &shelf.folder) {
            create(api, &libraries, &shelf, "public")?;
            created += 1;
        }
    }
    for account in accounts {
        let shelf = private(account);
        if !held(&libraries, &shelf.folder) {
            create(api, &libraries, &shelf, &account.id)?;
            created += 1;
        }
    }
    Ok(created)
}

#[derive(Debug, Default, PartialEq)]
struct Shelves {
    public: Vec<i64>,
    private: Vec<(String, i64)>,
}

impl Shelves {
    fn read(libraries: &Value) -> Self {
        let mut shelves = Shelves::default();
        for library in libraries.as_array().into_iter().flatten() {
            let Some(id) = library["id"].as_i64() else {
                continue;
            };
            let owners: Vec<&str> = folders(library).filter_map(owner).collect();
            if owners.is_empty() {
                shelves.public.push(id);
            }
            for name in owners {
                shelves.private.push((name.to_string(), id));
            }
        }
        shelves.public.sort_unstable();
        shelves
    }

    fn of(&self, user: &str) -> Vec<i64> {
        let user = user.to_lowercase();
        let mut ids = self.public.clone();
        ids.extend(
            self.private
                .iter()
                .filter(|(owner, _)| *owner == user)
                .map(|(_, id)| *id),
        );
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

fn granted(user: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = user["libraries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|library| library["id"].as_i64())
        .collect();
    ids.sort_unstable();
    ids
}

fn unrestricted() -> Value {
    json!({"ageRating": NO_AGE_RATING, "includeUnknowns": true})
}

fn grant(api: &impl Api, accounts: &[Account], libraries: &Value) -> Result<usize, String> {
    let shelves = Shelves::read(libraries);
    let users = api.get("/users")?;
    let mut changed = 0;
    for user in users.as_array().ok_or("Kavita sent no list of users")? {
        let Some(name) = user["username"].as_str() else {
            continue;
        };
        if !accounts
            .iter()
            .any(|account| account.id == name.to_lowercase())
        {
            continue;
        }
        let wanted = shelves.of(name);
        if granted(user) == wanted && user["ageRestriction"] == unrestricted() {
            continue;
        }
        let roles: Vec<&Value> = user["roles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|role| role.as_str() != Some("Pleb"))
            .collect();
        api.send(
            "POST",
            "/account/update",
            &json!({
                "userId": user["id"],
                "username": name,
                "email": user["email"],
                "roles": roles,
                "libraries": wanted,
                "ageRestriction": unrestricted(),
                "identityProvider": user["identityProvider"],
            }),
        )?;
        println!("Kavita {name}: set libraries and no age rating");
        changed += 1;
    }
    Ok(changed)
}

pub fn sign_in(authority: &str, libraries: &[i64]) -> Map<String, Value> {
    rules(json!({
        "authority": authority,
        "clientId": "kavita",
        "provisionAccounts": true,
        "requireVerifiedEmail": false,
        "syncUserSettings": false,
        "autoLogin": false,
        "disablePasswordAuthentication": false,
        "providerName": "Authelia",
        "defaultRoles": ROLES,
        "defaultLibraries": libraries,
        "defaultAgeRestriction": NO_AGE_RATING,
        "defaultIncludeUnknowns": true,
    }))
}

fn server(current: &Value, authority: &str, libraries: &[i64]) -> Map<String, Value> {
    let mut oidc = current["oidcConfig"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    oidc.extend(sign_in(authority, libraries));
    let mut wanted = rules(json!({"enableOpds": true, "coverImageSize": LARGEST_COVERS}));
    wanted.insert("oidcConfig".into(), Value::Object(oidc));
    wanted
}

pub fn enforce(api: &impl Api, accounts: &[Account], authority: &str) -> Result<usize, String> {
    let mut changed = shelve(api, accounts)?;
    let libraries = api.get("/library/libraries")?;
    let settings = api.get("/settings")?;
    let wanted = server(&settings, authority, &Shelves::read(&libraries).public);
    let changes = settle(api, "POST", "/settings", &settings, &wanted)?;
    changed += report("Kavita", "server", &[&changes]);
    changed += grant(api, accounts, &libraries)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    const AUTHORITY: &str = "https://auth.example.org:50000";

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
        }
    }

    fn library(id: i64, name: &str, folder: &str) -> Value {
        json!({"id": id, "name": name, "type": BOOK, "folders": [folder]})
    }

    fn libraries() -> Value {
        json!([
            library(1, "Livres", "/data/media/books"),
            library(2, "Mangas", "/data/media/manga/"),
            library(3, "Eléa", "/data/media/private/elea_t/books"),
            library(4, "Théo", "/data/media/private/theo/books"),
        ])
    }

    fn user(name: &str, ids: &[i64]) -> Value {
        let libraries: Vec<Value> = ids.iter().map(|id| json!({"id": id})).collect();
        json!({
            "id": 9,
            "username": name,
            "email": format!("{name}@jellynas.invalid"),
            "libraries": libraries,
            "roles": ["Pleb", "Login"],
            "ageRestriction": {"ageRating": -1, "includeUnknowns": true},
            "identityProvider": 1,
        })
    }

    fn in_line() -> BTreeMap<String, Value> {
        let mut pages = BTreeMap::new();
        pages.insert("/library/libraries".into(), libraries());
        let mut oidc = sign_in(AUTHORITY, &[1, 2]);
        oidc.insert("secret".into(), json!("********"));
        pages.insert(
            "/settings".into(),
            json!({"enableOpds": true, "coverImageSize": 4, "taskScan": "daily", "oidcConfig": oidc}),
        );
        pages.insert(
            "/users".into(),
            json!([
                user("admin", &[1, 2, 3, 4]),
                user("elea_t", &[1, 2, 3]),
                user("theo", &[2, 1, 4]),
            ]),
        );
        pages
    }

    fn accounts() -> Vec<Account> {
        vec![account("elea_t", "Eléa"), account("theo", "Théo")]
    }

    #[test]
    fn a_kavita_in_line_needs_nothing() {
        let fake = Fake::new(in_line());
        assert_eq!(Ok(0), enforce(&fake, &accounts(), AUTHORITY));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn the_public_libraries_are_created_with_their_type_and_files() {
        let mut pages = in_line();
        pages.insert("/library/libraries".into(), json!([]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(2), shelve(&fake, &[]));
        assert_eq!(vec!["POST /library/create"; 2], fake.paths());
        let books = fake.body(0);
        assert_eq!(json!("Livres"), books["name"]);
        assert_eq!(json!(BOOK), books["type"]);
        assert_eq!(json!(["/data/media/books"]), books["folders"]);
        assert_eq!(json!([EPUB, PDF]), books["fileGroupTypes"]);
        assert_eq!(json!(HARDCOVER), books["metadataProvider"]);
        let manga = fake.body(1);
        assert_eq!(json!(MANGA), manga["type"]);
        assert_eq!(json!([ARCHIVE]), manga["fileGroupTypes"]);
        assert_eq!(json!(MANGABAKA), manga["metadataProvider"]);
    }

    #[test]
    fn an_account_without_a_private_library_gets_one_on_its_books_folder() {
        let fake = Fake::new(in_line());
        let found = [
            account("elea_t", "Eléa"),
            account("nima", "Nima"),
            account("niixe", "Livres"),
        ];
        assert_eq!(Ok(2), shelve(&fake, &found));
        let nima = fake.body(0);
        assert_eq!(json!("Nima"), nima["name"]);
        assert_eq!(json!(["/data/media/private/nima/books"]), nima["folders"]);
        assert_eq!(json!([ARCHIVE, EPUB, PDF]), nima["fileGroupTypes"]);
        assert_eq!(json!("Livres (niixe)"), fake.body(1)["name"]);
    }

    #[test]
    fn a_private_library_belongs_to_the_account_named_by_its_folder() {
        let shelves = Shelves::read(&libraries());
        assert_eq!(vec![1, 2], shelves.public);
        assert_eq!(vec![1, 2, 3], shelves.of("Elea_T"));
        assert_eq!(vec![1, 2], shelves.of("nima"));
    }

    #[test]
    fn an_account_granted_another_private_library_loses_it() {
        let mut pages = in_line();
        pages.insert("/users".into(), json!([user("theo", &[1, 2, 3, 4])]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        assert_eq!(vec!["POST /account/update"], fake.paths());
        let sent = fake.body(0);
        assert_eq!(json!(9), sent["userId"]);
        assert_eq!(json!("theo"), sent["username"]);
        assert_eq!(json!("theo@jellynas.invalid"), sent["email"]);
        assert_eq!(json!(["Login"]), sent["roles"]);
        assert_eq!(json!([1, 2, 4]), sent["libraries"]);
        assert_eq!(json!(1), sent["identityProvider"]);
    }

    #[test]
    fn a_new_account_gets_its_libraries() {
        let mut pages = in_line();
        pages.insert("/users".into(), json!([user("elea_t", &[])]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        assert_eq!(json!([1, 2, 3]), fake.body(0)["libraries"]);
    }

    #[test]
    fn an_account_hidden_behind_an_age_rating_sees_everything_again() {
        let mut pages = in_line();
        let mut elea = user("elea_t", &[1, 2, 3]);
        elea["ageRestriction"] = json!({"ageRating": 0, "includeUnknowns": false});
        pages.insert("/users".into(), json!([elea]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        let sent = fake.body(0);
        assert_eq!(json!([1, 2, 3]), sent["libraries"]);
        assert_eq!(
            json!({"ageRating": -1, "includeUnknowns": true}),
            sent["ageRestriction"]
        );
    }

    #[test]
    fn new_accounts_start_without_an_age_rating() {
        let oidc = sign_in(AUTHORITY, &[1, 2]);
        assert_eq!(json!(-1), oidc["defaultAgeRestriction"]);
        assert_eq!(json!(true), oidc["defaultIncludeUnknowns"]);
    }

    #[test]
    fn a_user_outside_lldap_keeps_its_libraries() {
        let mut pages = in_line();
        pages.insert("/users".into(), json!([user("admin", &[3])]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(0), enforce(&fake, &accounts(), AUTHORITY));
    }

    #[test]
    fn sign_in_with_authelia_comes_back_and_keeps_the_secret() {
        let mut pages = in_line();
        pages.get_mut("/settings").unwrap()["oidcConfig"]["provisionAccounts"] = json!(false);
        pages.get_mut("/settings").unwrap()["oidcConfig"]["defaultLibraries"] = json!([1, 2, 3]);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        assert_eq!(vec!["POST /settings"], fake.paths());
        let sent = fake.body(0);
        assert_eq!(json!(true), sent["oidcConfig"]["provisionAccounts"]);
        assert_eq!(json!([1, 2]), sent["oidcConfig"]["defaultLibraries"]);
        assert_eq!(json!("********"), sent["oidcConfig"]["secret"]);
        assert_eq!(json!("daily"), sent["taskScan"]);
    }

    #[test]
    fn opds_turned_off_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut("/settings").unwrap()["enableOpds"] = json!(false);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        assert_eq!(json!(true), fake.body(0)["enableOpds"]);
    }

    #[test]
    fn covers_are_kept_at_the_largest_size() {
        let mut pages = in_line();
        pages.get_mut("/settings").unwrap()["coverImageSize"] = json!(1);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &accounts(), AUTHORITY));
        assert_eq!(json!(4), fake.body(0)["coverImageSize"]);
    }
}
