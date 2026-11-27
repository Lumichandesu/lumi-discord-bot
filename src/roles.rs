//! The self-service role panel: a Member toggle and an age-checked NSFW role.
//!
//! The name and birth year typed into the NSFW form are only checked, never stored.

use chrono::Datelike;
use serenity::all::{
    ActionRowComponent, ButtonStyle, ChannelId, ComponentInteraction, Context, CreateActionRow, CreateButton,
    CreateEmbed, CreateInputText, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
    CreateModal, GuildId, InputTextStyle, ModalInteraction, ReactionType, RoleId, UserId,
};

pub const TARGET_CHANNEL_ID: ChannelId = ChannelId::new(1542360532319871098);
const MEMBER_ROLE: RoleId = RoleId::new(1542375713594937364);
const NSFW_ROLE: RoleId = RoleId::new(1542375632225173545);

pub const BUTTON_PREFIX: &str = "btn_role_";
pub const NSFW_MODAL: &str = "modal_nsfw_verify";

const NO_GUILD: &str = "❌ ไม่สามารถใช้ระบบนี้ได้ในเซิร์ฟเวอร์นี้";

fn panel() -> CreateMessage {
    let embed = CreateEmbed::new().title("✧・รับยศเปิดห้อง・✧").description(
        "(づ￣ ³￣)づ⁄  กดปุ่มด้านล่างเพื่อรับยศและเปิดการมองเห็นห้องต่างๆ 👇\n\n\
         💎 **Member** : ปลดล็อกห้องพูดคุย\n\
         🔞 **NSFW (18+)** : ปลดล็อกห้องใต้เตียง",
    );
    let row = CreateActionRow::Buttons(vec![
        CreateButton::new("btn_role_member")
            .label("Member")
            .style(ButtonStyle::Primary)
            .emoji(ReactionType::Unicode("💎".into())),
        CreateButton::new("btn_role_nsfw")
            .label("NSFW (18+)")
            .style(ButtonStyle::Danger)
            .emoji(ReactionType::Unicode("🔞".into())),
    ]);
    CreateMessage::new().embed(embed).components(vec![row])
}

/// Posts the panel in the role channel.
pub async fn send_panel(ctx: &Context) -> Result<(), String> {
    TARGET_CHANNEL_ID.send_message(&ctx.http, panel()).await.map(|_| ()).map_err(|e| e.to_string())
}

fn reply(text: impl Into<String>) -> CreateInteractionResponse {
    CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().content(text).ephemeral(true))
}

async fn role_name(ctx: &Context, guild: GuildId, role: RoleId) -> Result<Option<String>, String> {
    if let Some(name) = ctx.cache.guild(guild).and_then(|g| g.roles.get(&role).map(|r| r.name.clone())) {
        return Ok(Some(name));
    }
    let roles = guild.roles(&ctx.http).await.map_err(|e| e.to_string())?;
    Ok(roles.get(&role).map(|r| r.name.clone()))
}

/// The member's current roles, from the interaction itself or, failing that, from Discord.
async fn member_roles(
    ctx: &Context,
    guild: GuildId,
    user: UserId,
    from_interaction: Option<&[RoleId]>,
) -> Result<Vec<RoleId>, String> {
    if let Some(roles) = from_interaction {
        return Ok(roles.to_vec());
    }
    let member = ctx.http.get_member(guild, user).await.map_err(|e| e.to_string())?;
    Ok(member.roles)
}

/// Handles `btn_role_*` buttons.
pub async fn on_button(ctx: &Context, i: &ComponentInteraction) {
    let response = match i.guild_id {
        None => reply(NO_GUILD),
        Some(guild) => match i.data.custom_id.strip_prefix(BUTTON_PREFIX) {
            Some("nsfw") => CreateInteractionResponse::Modal(nsfw_modal()),
            Some("member") => {
                let roles = i.member.as_ref().map(|m| m.roles.as_slice());
                match toggle_member(ctx, guild, i.user.id, roles).await {
                    Ok(text) => reply(text),
                    Err(e) => {
                        tracing::error!("[Role Button Error]: {e}");
                        reply("❌ เกิดข้อผิดพลาดขณะจัดการยศ โปรดลองอีกครั้ง")
                    }
                }
            }
            _ => reply("❌ ไม่พบยศที่ต้องการ"),
        },
    };
    if let Err(e) = i.create_response(&ctx.http, response).await {
        tracing::warn!("role button {} failed: {e}", i.data.custom_id);
    }
}

fn nsfw_modal() -> CreateModal {
    let name = CreateInputText::new(InputTextStyle::Short, "Name", "nsfw_name")
        .placeholder("Enter your name")
        .required(true)
        .min_length(1)
        .max_length(100);
    let year = CreateInputText::new(InputTextStyle::Short, "Birth Year", "nsfw_birth_year")
        .placeholder("Example: 2000")
        .required(true)
        .min_length(4)
        .max_length(4);
    CreateModal::new(NSFW_MODAL, "🔞 NSFW (18+) Verification")
        .components(vec![CreateActionRow::InputText(name), CreateActionRow::InputText(year)])
}

async fn toggle_member(ctx: &Context, guild: GuildId, user: UserId, roles: Option<&[RoleId]>) -> Result<String, String> {
    let Some(name) = role_name(ctx, guild, MEMBER_ROLE).await? else {
        return Ok("❌ ไม่พบยศนี้ในเซิร์ฟเวอร์".into());
    };
    let Ok(current) = member_roles(ctx, guild, user, roles).await else {
        return Ok("❌ ไม่พบข้อมูลสมาชิก".into());
    };
    if current.contains(&MEMBER_ROLE) {
        ctx.http.remove_member_role(guild, user, MEMBER_ROLE, None).await.map_err(|e| e.to_string())?;
        Ok(format!("🗑️ ถอดยศ **{name}** ออกเรียบร้อยแล้ว"))
    } else {
        ctx.http.add_member_role(guild, user, MEMBER_ROLE, None).await.map_err(|e| e.to_string())?;
        Ok(format!("✨ มอบยศ **{name}** ให้เรียบร้อยแล้ว!"))
    }
}

/// Checks the form. Returns the trimmed name and the age, or the reply to show.
pub fn check_age(name: &str, birth_year: &str, current_year: i32) -> Result<(String, i32), &'static str> {
    let name = name.trim();
    let birth_year = birth_year.trim();
    if name.is_empty() {
        return Err("❌ กรุณากรอกชื่อ");
    }
    if birth_year.len() != 4 || !birth_year.bytes().all(|b| b.is_ascii_digit()) {
        return Err("❌ กรุณากรอกปีเกิดเป็นตัวเลข 4 หลัก เช่น `2000`");
    }
    let year: i32 = birth_year.parse().map_err(|_| "❌ กรุณากรอกปีเกิดเป็นตัวเลข 4 หลัก เช่น `2000`")?;
    if year < 1900 || year > current_year {
        return Err("❌ ปีเกิดไม่ถูกต้อง กรุณาตรวจสอบอีกครั้ง");
    }
    // Birth year only, as the server owner asked: in 2026, 2008 counts as 18.
    let age = current_year - year;
    if age < 18 {
        return Err("🚫 **ไม่ผ่านการยืนยันอายุ**\n\nระบบอนุญาตเฉพาะสมาชิกที่มีอายุ 18 ปีขึ้นไปเท่านั้น");
    }
    Ok((name.to_string(), age))
}

fn field<'a>(i: &'a ModalInteraction, id: &str) -> &'a str {
    i.data
        .components
        .iter()
        .flat_map(|row| &row.components)
        .find_map(|c| match c {
            ActionRowComponent::InputText(t) if t.custom_id == id => t.value.as_deref(),
            _ => None,
        })
        .unwrap_or("")
}

/// Handles the NSFW form. Returns `false` for other modals.
pub async fn on_modal(ctx: &Context, i: &ModalInteraction) -> bool {
    if i.data.custom_id != NSFW_MODAL {
        return false;
    }
    let text = match i.guild_id {
        None => NO_GUILD.to_string(),
        Some(guild) => {
            let roles = i.member.as_ref().map(|m| m.roles.as_slice());
            match verify(ctx, guild, i.user.id, roles, field(i, "nsfw_name"), field(i, "nsfw_birth_year")).await {
                Ok(text) => text,
                Err(e) => {
                    tracing::error!("[NSFW Verification Error]: {e}");
                    "❌ เกิดข้อผิดพลาดระหว่างการยืนยัน โปรดลองอีกครั้ง".into()
                }
            }
        }
    };
    if let Err(e) = i.create_response(&ctx.http, reply(text)).await {
        tracing::warn!("NSFW form reply failed: {e}");
    }
    true
}

async fn verify(
    ctx: &Context,
    guild: GuildId,
    user: UserId,
    roles: Option<&[RoleId]>,
    name: &str,
    birth_year: &str,
) -> Result<String, String> {
    let (name, age) = match check_age(name, birth_year, chrono::Local::now().year()) {
        Ok(ok) => ok,
        Err(text) => return Ok(text.into()),
    };
    let Some(role) = role_name(ctx, guild, NSFW_ROLE).await? else {
        return Ok("❌ ไม่พบ NSFW role ในเซิร์ฟเวอร์ กรุณาติดต่อผู้ดูแล".into());
    };
    if member_roles(ctx, guild, user, roles).await?.contains(&NSFW_ROLE) {
        return Ok("✅ คุณมี NSFW role อยู่แล้ว\n\nคุณผ่านการยืนยันอายุแล้ว".into());
    }
    ctx.http.add_member_role(guild, user, NSFW_ROLE, None).await.map_err(|e| e.to_string())?;
    Ok(format!(
        "✅ **Age verification successful**\n\nยินดีต้อนรับ {name} 🎉\nคุณผ่านการยืนยันอายุแล้ว ({age}+) \n\n🔞 มอบยศ **{role}** ให้เรียบร้อยแล้ว"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_check_follows_birth_year_only() {
        assert_eq!(check_age(" Lumi ", "2008", 2026), Ok(("Lumi".into(), 18)));
        assert_eq!(check_age("a", "1900", 2026), Ok(("a".into(), 126)));
        assert!(check_age("a", "2009", 2026).unwrap_err().starts_with("🚫"));
        assert_eq!(check_age("  ", "2000", 2026), Err("❌ กรุณากรอกชื่อ"));
        assert!(check_age("a", "20x0", 2026).unwrap_err().contains("4 หลัก"));
        assert!(check_age("a", "200", 2026).unwrap_err().contains("4 หลัก"));
        assert!(check_age("a", "١٢٣٤", 2026).unwrap_err().contains("4 หลัก"));
        assert_eq!(check_age("a", "1899", 2026), Err("❌ ปีเกิดไม่ถูกต้อง กรุณาตรวจสอบอีกครั้ง"));
        assert_eq!(check_age("a", "2027", 2026), Err("❌ ปีเกิดไม่ถูกต้อง กรุณาตรวจสอบอีกครั้ง"));
    }
}
