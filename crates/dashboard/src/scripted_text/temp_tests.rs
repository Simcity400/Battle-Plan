use super::*;

#[test]
fn temp_variables_drive_localisation_conditions_and_text_weights() {
    let (catalogue, _folder) = catalogue(
        r#"
        a = { name = Pick text = {
            trigger = { set_temp_variable = { t = 2 }
                multiply_temp_variable = { t = 3 }
                check_variable = { t = 6 }
                modulo_temp_variable = { t = 4 }
                check_variable = { t = 2 } }
            random_list = { seed = t
                1 = { localization_key = YES modifier = { factor = t } }
                0 = { localization_key = NO } }
        } }
        a = { name = Reset
            text = { trigger = { set_temp_variable = { t = 9 } always = no }
                localization_key = NO }
            text = { trigger = { check_variable = { temp_var:t = 0 } }
                localization_key = YES }
        }
        "#,
        " YES:0 \"selected\"\n NO:0 \"wrong\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    assert!(
        content
            .environment
            .set(Ok(entries::Environment::synthetic()))
            .is_ok()
    );
    let mut frame = frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 0);
    assert_eq!(
        text.expand("[Pick]/[Reset]").as_deref(),
        Ok("selected/selected")
    );
}
