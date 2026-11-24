use crate::api::{Api, Http};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub const PRIVATE_ROOT: &str = "/data/media/private";
const SERVICE_ACCOUNT: &str = "policy";
const QUERY: &str = "{ users(filters: {memberOf: \"jellynas_users\"}) { id displayName } }";

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: String,
    pub name: String,
}

impl Account {
    pub fn folder(&self) -> String {
        format!("{PRIVATE_ROOT}/{}", self.id)
    }
}

pub struct Directory {
    pub base: String,
    pub password: String,
}

impl Directory {
    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        let login = Http {
            base: self.base.clone(),
            header: None,
        };
        let token = token(&login, &self.password)?;
        let api = Http {
            base: self.base.clone(),
            header: Some(("Authorization", format!("Bearer {token}"))),
        };
        members(&api)
    }
}

pub fn token(api: &impl Api, password: &str) -> Result<String, String> {
    let reply = api.post(
        "/auth/simple/login",
        &json!({"username": SERVICE_ACCOUNT, "password": password}),
    )?;
    reply["token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "lldap sent no token".into())
}

pub fn members(api: &impl Api) -> Result<Vec<Account>, String> {
    let reply = api.post("/api/graphql", &json!({"query": QUERY}))?;
    let users = reply["data"]["users"]
        .as_array()
        .ok_or("lldap sent no list of users")?;
    let mut accounts: Vec<Account> = users.iter().filter_map(account).collect();
    accounts.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(accounts)
}

fn account(user: &Value) -> Option<Account> {
    let id = user["id"].as_str().filter(|id| safe(id))?;
    let name = user["displayName"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(id);
    Some(Account {
        id: id.to_string(),
        name: name.to_string(),
    })
}

fn safe(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c))
}

pub fn shelve(root: &Path, accounts: &[Account]) -> Result<usize, String> {
    let mut created = 0;
    for account in accounts {
        let folder = root.join(&account.id);
        if folder.is_dir() {
            continue;
        }
        fs::create_dir(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        restrict(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        println!("Private folders: created {}", account.id);
        created += 1;
    }
    Ok(created)
}

#[cfg(unix)]
fn restrict(folder: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o770))
}

#[cfg(not(unix))]
fn restrict(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    fn eléa() -> Account {
        Account {
            id: "elea_t".into(),
            name: "Eléa".into(),
        }
    }

    #[test]
    fn the_service_account_signs_in() {
        let mut pages = BTreeMap::new();
        pages.insert("/auth/simple/login".into(), json!({"token": "t"}));
        let fake = Fake::new(pages);
        assert_eq!(Ok("t".into()), token(&fake, "secret"));
        assert_eq!(json!("policy"), fake.body(0)["username"]);
        assert_eq!(json!("secret"), fake.body(0)["password"]);
    }

    #[test]
    fn only_members_of_jellynas_users_are_read() {
        let mut pages = BTreeMap::new();
        pages.insert(
            "/api/graphql".into(),
            json!({"data": {"users": [
                {"id": "moutondu", "displayName": "Vincent"},
                {"id": "elea_t", "displayName": "Eléa"},
            ]}}),
        );
        let fake = Fake::new(pages);
        let accounts = members(&fake).unwrap();
        assert!(
            fake.body(0)["query"]
                .as_str()
                .unwrap()
                .contains("jellynas_users")
        );
        assert_eq!(
            vec!["elea_t", "moutondu"],
            accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>()
        );
        assert_eq!("/data/media/private/elea_t", accounts[0].folder());
    }

    #[test]
    fn an_account_without_display_name_is_named_after_its_id() {
        assert_eq!(
            Some(Account {
                id: "theo".into(),
                name: "theo".into()
            }),
            account(&json!({"id": "theo", "displayName": " "}))
        );
    }

    #[test]
    fn an_id_that_could_leave_the_private_root_is_skipped() {
        for id in ["", "..", ".hidden", "a/b", "a b"] {
            assert_eq!(None, account(&json!({"id": id, "displayName": "x"})));
        }
    }

    #[test]
    fn a_missing_folder_is_created_once() {
        let root = std::env::temp_dir().join(format!("policy-shelve-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert_eq!(Ok(1), shelve(&root, &[eléa()]));
        assert!(root.join("elea_t").is_dir());
        assert_eq!(Ok(0), shelve(&root, &[eléa()]));
        fs::remove_dir_all(&root).unwrap();
    }
}
