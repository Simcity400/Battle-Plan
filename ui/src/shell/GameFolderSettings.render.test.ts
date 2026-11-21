import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('a folder failure lands on the folder it names, and a failure to save the pair stays at form level', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: GameFolderSettings, FolderField, folderFailure } = await vite.ssrLoadModule('/ui/src/shell/GameFolderSettings.tsx') as typeof import('./GameFolderSettings');
    const { default: ReplayFolderSetting } = await vite.ssrLoadModule('/ui/src/shell/ReplayFolderSetting.tsx') as typeof import('./ReplayFolderSetting');
    assert.deepEqual(folderFailure('Choose the game folder that contains hoi4.exe.'),
      { field: 'game', text: 'Choose the folder the game is installed in.', detail: 'Choose the game folder that contains hoi4.exe.' },
      'a wrong game folder marks Game installation, even when the Paradox user folder was chosen last, without showing a file name');
    assert.deepEqual(folderFailure('This Paradox user folder doesn’t exist.'), { field: 'user', text: 'This Paradox user folder doesn’t exist.' });
    assert.equal(folderFailure('Choose game folder and the Paradox user folder.').field, null, 'a sentence about both folders marks neither');
    assert.equal(folderFailure('Choose the game and user folders.').field, null);
    assert.deepEqual(folderFailure('Couldn’t open GER.battleplan.', 'Couldn’t use this folder.'), { field: null, text: 'Couldn’t use this folder.', detail: 'Couldn’t open GER.battleplan.' },
      'a file name never reaches the visible sentence, as everywhere else in the app');
    assert.deepEqual(folderFailure('Access is denied. (os error 5)'), { field: null, text: 'Couldn’t save these folders.', detail: 'Access is denied. (os error 5)' },
      'an operating-system reason stays out of the visible text');
    const unread = folderFailure('Couldn’t read the recordings folder: Access is denied. (os error 5)', 'Couldn’t use this folder.');
    assert.equal(unread.field, null);
    assert.doesNotMatch(unread.text, /os error|:/);
    assert.equal(unread.detail, 'Couldn’t read the recordings folder: Access is denied. (os error 5)');
    assert.deepEqual(folderFailure('stream did not contain valid UTF-8', 'Couldn’t use this folder.'), { field: null, text: 'Couldn’t use this folder.', detail: 'stream did not contain valid UTF-8' });
    const pathed = 'D:\\Recordings\\Germany 1936: Access is denied. (os error 5)';
    assert.deepEqual(folderFailure(pathed, 'Couldn’t use this folder.'), { field: null, text: 'Couldn’t use this folder.', detail: pathed },
      'a path before the colon is never shown as the sentence');
    assert.deepEqual(folderFailure('os path missing: no such entry', 'Couldn’t use this folder.'), { field: null, text: 'Couldn’t use this folder.', detail: 'os path missing: no such entry' },
      'a lower-case fragment is never shown as the sentence');

    const field = (error: string | null) => renderToStaticMarkup(createElement(FolderField, { label: 'Game installation', help: 'Where the game is installed.', value: 'C:/Games',
      placeholder: 'Choose folder', disabled: false, error: error ? folderFailure(error) : null, onChange() {}, onCommit() {} }));
    const wrong = field('Choose the game folder that contains hoi4.exe.');
    assert.match(wrong, /aria-invalid="true" aria-describedby="game-installation-note"/);
    assert.match(wrong, /id="game-installation-note"><p class="failure-line" role="alert" data-detail="[^"]*"><svg[^]*?<span>Choose the folder the game is installed in\.<\/span><\/p>/,
      'the field error is the shared failure line');
    const plain = renderToStaticMarkup(createElement(FolderField, { label: 'Game installation', help: 'Where the game is installed.', value: 'D:/Games/Run (2)/',
      placeholder: 'Choose folder', disabled: false, onChange() {}, onCommit() {} }));
    assert.match(plain, /<input aria-label="Game installation"[^>]* value="D:\/Games\/Run \(2\)\/"\/>/,
      'the field holds exactly the path, so selecting and typing over it replaces it instead of appending to a hidden copy');
    assert.doesNotMatch(plain, /[\u2066\u2069]/, 'no direction marks enter the value');
    assert.match(plain, /class="folder-path-shown" aria-hidden="true"><bdi>D:\/Games\/Run \(2\)\/<\/bdi>/, 'the left-clipped path is drawn over the unfocused field');
    const raw = field('Access is denied. (os error 5)');
    assert.match(raw, /data-detail="Access is denied\. \(os error 5\)"/);
    assert.doesNotMatch(raw.replace(/data-detail="[^"]*"/g, ''), /os error/);
    assert.doesNotMatch(field(null), /aria-invalid|failure-line/, 'the correct field stays clean');

    const folders = (unread: boolean) => renderToStaticMarkup(createElement(GameFolderSettings, { folders: null, unread, client: {} as never, onFoldersChange() {} }));
    assert.match(folders(false), /placeholder="Reading configured folders…"/);
    assert.doesNotMatch(folders(true), /Reading configured folders/, 'after a failed read the fields don’t claim to be reading');
    assert.match(folders(true), /placeholder="Choose folder"/);
    const recordings = (error: string) => renderToStaticMarkup(createElement(ReplayFolderSetting, { client: {} as never,
      library: { folder: 'D:/Recordings', error, loading: false, refresh: async () => {} } as never }));
    const refused = recordings('This folder is too large. Choose a folder that holds only your recordings.');
    assert.match(refused, /<span>This folder is too large\. Choose a folder that holds only your recordings\.<\/span>/);
    assert.doesNotMatch(refused, /Try again/, 'a refusal of the folder itself isn’t offered again unchanged');
    const unreadable = recordings('Access is denied. (os error 5)');
    assert.match(unreadable, /<span>Couldn’t read the recordings folder\.<\/span><button[^>]*>Try again<\/button>/, 'a failure to read the folder can be tried again');
  } finally {
    await vite.close();
  }
});
