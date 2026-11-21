# 0201. Keep an early army-icon capture through Late

Army icon inputs are captured from the shared authenticated division census and seeded unit inputs. When the dedicated job runs just after the division sources join, its rows and worker time belong to the current hour. The Late capture keeps them even if other Late work fails. Late withholds army-icon rows only when it owns that read. A vacant worker placeholder carries no capture allocation. An already refused field of the first row still names the failure if a later seed is missing.

[Army icon inputs](../../crates/collector-army-icons/src/army_icon_inputs.rs)
