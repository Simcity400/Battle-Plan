# Job: class (one class at a time)

Read `RULES.md` first. The item is a class whose own virtual functions are still called `vfN`.

1. `hoi4re class <Name>` and `hoi4re vtable <Name>`. Note the bases. Look at a named sibling class for the slot meanings
   (for a `C...Effect` look at other effects; for a `C...Command` look at `CHourlyTickCommand`).
2. For every slot whose function is named `<Name>::vfN`, read it with `fn` and name it. When you name a slot, the tool
   gives the same method name to every override in related classes, so get the method name right: check two sibling
   classes before saving a slot name that would spread.
3. Look for the save and load functions (they pair string keys or token ids with `this+offset`). Record each pair as a
   `field` line: the save key is the field name.
4. Save one `note` on the class vtable (`hoi4re find <Name>::vftable` gives its address) with two or three sentences:
   what the class is, which slots matter.
5. Close the item: `hoi4re todo done class <Name> --note "N slots named"`.
