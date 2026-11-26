import { useCallback, useEffect, useState } from 'react';
import { Icon } from '../components/Icons';
import { Select } from '../components/Select';
import { FailureLine } from '../components/FailureLine';
import { SettingsBlock } from '../shell/SettingsBlock';
import { desktopAvailable, errorMessage } from '../shell/api';
import {
  chooseFfmpegBinary,
  getExportSettings,
  probeExportBackends,
  setExportSettings,
  type BackendAvailability,
  type BackendChoice,
  type ExportSettings,
} from './videoExportApi';

export default function ExportSettingsBlock() {
  const [settings, setSettings] = useState<ExportSettings>({ backend: 'auto', ffmpegPath: null });
  const [ffmpegDraft, setFfmpegDraft] = useState('');
  const [availability, setAvailability] = useState<BackendAvailability | null>(null);
  const [loading, setLoading] = useState(desktopAvailable);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshProbe = useCallback(async (ffmpegPath?: string | null) => {
    if (!desktopAvailable) return;
    try {
      const probed = await probeExportBackends(ffmpegPath);
      setAvailability(probed);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  useEffect(() => {
    if (!desktopAvailable) return;
    let cancelled = false;
    setLoading(true);
    void getExportSettings()
      .then(async loaded => {
        if (cancelled) return;
        setSettings(loaded);
        setFfmpegDraft(loaded.ffmpegPath ?? '');
        await refreshProbe(loaded.ffmpegPath);
      })
      .catch(cause => {
        if (!cancelled) setError(errorMessage(cause));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshProbe]);

  const commit = async (next: ExportSettings) => {
    if (!desktopAvailable) return;
    setSaving(true);
    setError(null);
    try {
      const saved = await setExportSettings(next);
      setSettings(saved);
      setFfmpegDraft(saved.ffmpegPath ?? '');
      await refreshProbe(saved.ffmpegPath);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const browseFfmpeg = async () => {
    setError(null);
    try {
      const picked = await chooseFfmpegBinary();
      if (picked) {
        await commit({ ...settings, ffmpegPath: picked });
      }
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <SettingsBlock id="export-encoding" title="Video export encoding">
      {!desktopAvailable ? (
        <div className="settings-row">
          <span className="settings-row-text">
            <strong>MP4 encoder</strong>
            <small>Video export settings are available in the BattlePlan desktop app.</small>
          </span>
        </div>
      ) : (
        <>
          <div className="settings-row">
            <span className="settings-row-text">
              <strong>Encoder backend</strong>
              <small>
                Auto uses built-in Windows Media Foundation when available and falls back to FFmpeg.
              </small>
            </span>
            <div className="settings-row-control">
              <Select
                label="Video encoder backend"
                value={settings.backend}
                disabled={loading || saving}
                onChange={value => void commit({ ...settings, backend: value as BackendChoice })}
                options={[
                  { value: 'auto', label: 'Auto (Recommended)' },
                  { value: 'mediaFoundation', label: 'Windows Media Foundation' },
                  { value: 'ffmpeg', label: 'FFmpeg' },
                ]}
              />
            </div>
          </div>
          <div className="folder-setting-row">
            <span className="folder-field-title">FFmpeg executable</span>
            <div className="folder-path">
              <input
                type="text"
                aria-label="FFmpeg executable path"
                placeholder="Auto-detect from PATH"
                value={ffmpegDraft}
                disabled={loading || saving}
                onChange={event => setFfmpegDraft(event.currentTarget.value)}
                onBlur={() => {
                  const trimmed = ffmpegDraft.trim() || null;
                  if (trimmed !== settings.ffmpegPath) {
                    void commit({ ...settings, ffmpegPath: trimmed });
                  }
                }}
              />
            </div>
            <button
              type="button"
              className="button small folder-browse"
              aria-label="Browse for FFmpeg executable"
              data-tooltip="Browse for FFmpeg executable"
              disabled={loading || saving}
              onClick={() => void browseFfmpeg()}
            >
              <Icon name="folder-open" />
            </button>
            <div className="folder-note">
              {availability && (
                <p className="folder-hint">
                  Media Foundation: {availability.mediaFoundation ? 'Available' : 'Unavailable'} ({availability.mediaFoundationDetail}).
                  {' '}FFmpeg: {availability.ffmpeg ? 'Available' : 'Not found'} ({availability.ffmpegDetail}).
                </p>
              )}
            </div>
          </div>
          {error && <FailureLine className="settings-failure" message="Couldn’t update export encoding settings." detail={error} />}
        </>
      )}
    </SettingsBlock>
  );
}
