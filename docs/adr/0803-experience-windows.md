# Publish experience on the recorded daily read

`CountryExperienceDaily` tracks army, navy and air experience by raw country ID. Banked change compares the current read with the read exactly 24 hours earlier. Gross gains use all 24 hours ending on the current read. Every hour in that window needs exactly one complete gains interval for the country. An unavailable table or ambiguous or incomplete interval starts a new window so the reply never presents a partial sum as complete.

Use the `CountryExperience` sample hour to publish. A changed banked cell without a read stamp does not write a daily row. The first hour and a return after an hour the writer did not run start fresh, with today's values withheld unless the hour read the table. Persist the recent banked reads and runs of equal hourly gains in the recorder blob and rebuild the index of gain rows on resume.

[Writer](../../crates/recorder-derive-land/src/experience.rs)
