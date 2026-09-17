# Job: sourced (functions whose source file is known)

Read `RULES.md` first. The item's source file comes from assert strings inside the function, so the subsystem is known.

For each item from `hoi4re todo next sourced`:
1. `hoi4re fn <addr> --max-lines 150`. Read the assert and log strings: they quote real conditions and variable names
   (`"pCountry != nullptr"`, `"Error reading ..."`), and sometimes the function's purpose.
2. `hoi4re find <file name>` lists other functions of the same file; named neighbours give the class and style.
3. Name it. The class is usually the file's class (`country.cpp` -> `CCountry`), but only use it when the function takes
   that object as `this`.
4. Quote the deciding string in `evidence`.
