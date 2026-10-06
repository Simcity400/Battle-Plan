use super::*;

impl Renderer<'_, '_, '_> {
    fn folder_name(&self, value: &[u8]) -> Result<String, String> {
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database.folder(value)?.ok_or_else(|| {
            format!(
                "doctrine folder {} is absent",
                String::from_utf8_lossy(value)
            )
        })?;
        self.context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())
    }

    // 14051D460 -> 14051E8B0; only the reached folder selector.
    pub(super) fn mastery(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        for field in fields {
            if !matches!(
                key(field).to_ascii_lowercase().as_str(),
                "amount" | "folder"
            ) {
                return Err(format!(
                    "mastery filter {} is outside reached folder form (14051E8B0)",
                    key(field)
                ));
            }
        }
        let amount = field(fields, "amount")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let folder = field(fields, "folder")
            .and_then(|item| item.value.scalar())
            .ok_or("mastery folder form absent (14051E8B0)")?;
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database
            .folder_exact(folder)?
            .ok_or("mastery folder is absent (14051D460)")?;
        let name = self
            .context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())?;
        let effect = self.localised(
            b"ADD_MASTERY_EFFECT",
            &[("AMOUNT", Argument::Fixed5(amount))],
            scopes,
            false,
        )?;
        self.localised(
            b"TRACK_EFFECT_ONLY_FOLDER",
            &[
                ("TRACK_EFFECT_DESC", Argument::Text(&effect)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            false,
        )
    }

    // 14031FCD0: folder display, authored reduction, formatted use count.
    pub(super) fn doctrine_reduction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        // Native +0x58 technology and +0x70 category lists precede +0x88
        // folders (14031FCD0); never silently omit a technology operand.
        if let Some(technology) = field(fields, "technology") {
            return Err(format!(
                "technology {} doctrine reduction description is not ported (14031FCD0 +0x58)",
                String::from_utf8_lossy(technology.value.scalar().unwrap_or_default())
            ));
        }
        let reduction = field(fields, "cost_reduction")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let uses = field(fields, "uses")
            .and_then(|item| item.value.scalar())
            .map(installed_content::fixed5::decimal_i32)
            .unwrap_or(1);
        let mut folders = Vec::new();
        for item in fields.iter().filter(|item| item.keyword_is("category")) {
            folders.push(self.folder_name(item.value.scalar().unwrap_or_default())?);
        }
        if folders.is_empty() {
            return Err("non-folder doctrine reduction is outside reached form (14031FCD0)".into());
        }
        let list = match folders.as_slice() {
            [one] => one.clone(),
            _ => {
                let last = folders.pop().unwrap();
                format!(
                    "{} {} {last}",
                    folders.join(", "),
                    self.context
                        .localisation
                        .localise(b"AND") // Native literal at 14273BBDC, used by 14225CBD0.
                        .map_err(|why| why.to_string())?
                )
            }
        };
        let uses = self.localised(
            b"EFFECT_TECH_BONUS_USES",
            &[("USES", Argument::Int32(uses))],
            scopes,
            false,
        )?;
        self.line(
            b"EFFECT_ADD_DOCTRINE_COST_REDUCTION",
            &[
                ("LIST", Argument::Text(&list)),
                ("REDUCTION", Argument::Fixed5(reduction)),
                ("USES", Argument::Text(&uses)),
            ],
            scopes,
            false,
        )
    }
}
