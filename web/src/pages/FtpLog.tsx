// The FTP upload log: every workbook ALM dropped, who dropped or published it, which day it was
// for, and who replaced it with a later workbook. Every desk sees the published ones and can
// download the file exactly as it arrived; ALM also sees workbooks dropped and never published.
import { useState } from 'react';
import { endpoints } from '../api/client';
import { useFtpLog } from '../api/queries';
import type { FtpLogEntry, FtpReplacement } from '../api/types';
import { fmtDateShort, fmtLocal } from '../lib/fmt';
import { Badge, Chip } from '../components/ui';
import { useToast } from '../components/feedback';

const who = (by: string | null) => by ?? 'the application';

function size(n: number | null): string {
  if (n === null) return '';
  return n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${Math.round(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function replacement(r: FtpReplacement): string {
  return `${who(r.by)}${r.file_name ? `'s ${r.file_name}` : ''}, ${fmtLocal(r.at)}`;
}

function Status({ e }: { e: FtpLogEntry }) {
  switch (e.status) {
    case 'in_force':
      return <Badge tone="good">in force</Badge>;
    case 'published':
      return <Badge>published</Badge>;
    case 'replaced':
      return <Badge tone="warn">replaced</Badge>;
    case 'not_published':
      return e.errors > 0 ? <Badge tone="bad">{e.errors === 1 ? '1 error' : `${e.errors} errors`}, not published</Badge> : <Badge>not published</Badge>;
  }
}

function Detail({ e }: { e: FtpLogEntry }) {
  if (e.replaced_by) return <span className="t-small">by {replacement(e.replaced_by)}</span>;
  if (e.replaces) return <span className="t-small">replacing {replacement(e.replaces)}</span>;
  if (e.status === 'not_published') return <span className="t-small">dropped {e.uploaded_at ? fmtLocal(e.uploaded_at) : ''}</span>;
  return null;
}

export function FtpUploadLog({ testId = 'ftp-log' }: { testId?: string }) {
  const [limit, setLimit] = useState(25);
  const [filesOnly, setFilesOnly] = useState(true);
  const q = useFtpLog(limit, filesOnly);
  const toast = useToast();
  const entries = q.data?.entries ?? [];
  const get = (e: FtpLogEntry) =>
    e.upload_id !== null && endpoints.ftpFile(e.upload_id, e.file_name ?? 'ftp-workbook.xlsx').catch((err: Error) => toast({ tone: 'bad', title: 'The download failed', detail: err.message }));

  return (
    <div data-testid={testId}>
      <div className="row" style={{ gap: 4, marginBottom: 6 }}>
        <Chip small selected={filesOnly} onClick={() => setFilesOnly(true)}>
          Workbooks
        </Chip>
        <Chip small selected={!filesOnly} onClick={() => setFilesOnly(false)} title="Also the daily curves the application generated for the synthetic history">
          With the application's curves
        </Chip>
      </div>
      {q.isLoading ? (
        <div className="t-muted" style={{ padding: 12 }}>
          loading…
        </div>
      ) : entries.length === 0 ? (
        <div className="t-muted" style={{ padding: 12 }}>
          No workbook has been dropped yet.
        </div>
      ) : (
        <table className="table ftp-log">
          <thead>
            <tr>
              <th>When</th>
              <th>Who</th>
              <th>Workbook</th>
              <th>For</th>
              <th>Currencies</th>
              <th>Status</th>
              <th>Note</th>
            </tr>
          </thead>
          <tbody>
            {entries.map((e) => (
              <tr key={`${e.publication_id ?? 'u'}-${e.upload_id ?? 'p'}`} className={e.status === 'replaced' || e.status === 'not_published' ? 'dim' : ''} data-status={e.status}>
                <td className="nowrap">{fmtLocal(e.at)}</td>
                <td>
                  {who(e.by)}
                  {e.via === 'command line' && <div className="t-small">from the command line</div>}
                </td>
                <td>
                  {e.has_file ? (
                    <button type="button" className="link" onClick={() => void get(e)} title="Download the workbook as it was dropped">
                      {e.file_name}
                    </button>
                  ) : (
                    <span>{e.file_name ?? <span className="t-muted">no file</span>}</span>
                  )}
                  {e.has_file && (
                    <div className="t-small" title={e.file_sha256 ?? ''}>
                      {size(e.size_bytes)} · {e.file_sha256?.slice(0, 8)}
                    </div>
                  )}
                </td>
                <td className="nowrap">{e.as_of ? fmtDateShort(e.as_of) : <span className="t-muted">—</span>}</td>
                <td>{e.currencies.join(', ')}</td>
                <td>
                  <Status e={e} />
                  <div>
                    <Detail e={e} />
                  </div>
                </td>
                <td className="t-small ellipsis" style={{ maxWidth: 320 }} title={e.note ?? ''}>
                  {e.note ?? ''}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {q.data?.more && (
        <button type="button" className="btn btn-ghost btn-sm" style={{ marginTop: 6 }} onClick={() => setLimit((l) => l + 50)}>
          Show older entries
        </button>
      )}
    </div>
  );
}
