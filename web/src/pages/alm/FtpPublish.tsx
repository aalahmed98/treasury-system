// ALM publishes the day's FTP curves: drop the workbook, see what it holds, what is wrong with it
// (cell by cell) and how it differs from the curves in force, then publish it with a note. Nothing
// is stored until Publish; every desk sees the curves the moment it is pressed.
import { useRef, useState } from 'react';
import { Link } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ApiError, endpoints } from '../../api/client';
import { useFtp } from '../../api/queries';
import type { FtpChange, FtpPreview } from '../../api/types';
import { fmtDateLong, fmtDateShort, fmtLocal } from '../../lib/fmt';
import { Badge, Section } from '../../components/ui';
import { useToast } from '../../components/feedback';
import { FtpCurvesChart, FtpTable } from '../Ftp';
import { FtpUploadLog } from '../FtpLog';

function describe(c: FtpChange): string {
  switch (c.kind) {
    case 'added':
      return `${c.ccy}: a new curve`;
    case 'missing':
      return `${c.ccy}: not in this workbook, so it is retired from this date (bonds already held keep the rate they locked)`;
    case 'moved': {
      const bp = Math.round((c.to - c.from) * 100);
      return `${c.ccy} ${c.tenor}: ${c.from.toFixed(3)}% to ${c.to.toFixed(3)}% (${bp > 0 ? '+' : '−'}${Math.abs(bp)} bp)`;
    }
    case 'tenors':
      return `${c.ccy}: ${[c.added.length ? `adds ${c.added.join(', ')}` : '', c.removed.length ? `drops ${c.removed.join(', ')}` : ''].filter(Boolean).join('; ')}`;
  }
}

export function FtpPublish() {
  const ftp = useFtp();
  const qc = useQueryClient();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState<FtpPreview | null>(null);
  const [asOf, setAsOf] = useState('');
  const [note, setNote] = useState('');
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const read = useMutation({
    mutationFn: (f: File) => endpoints.ftpPreview(f),
    onSuccess: (p) => {
      setPreview(p);
      setAsOf(p.as_of);
      setError(null);
      void qc.invalidateQueries({ queryKey: ['ftp', 'log'] });
    },
    onError: (e) => {
      setPreview(null);
      setError(e instanceof ApiError ? e.message : 'the workbook could not be read');
    },
  });
  const publish = useMutation({
    mutationFn: () =>
      endpoints.ftpPublish({ as_of: asOf, curves: preview!.curves, file_name: file?.name ?? null, file_sha256: preview!.file_sha256, note: note.trim(), upload_id: preview!.upload_id }),
    onSuccess: (p) => {
      void qc.invalidateQueries({ queryKey: ['ftp'] });
      void qc.invalidateQueries({ queryKey: ['pmu'] });
      toast({ tone: 'good', title: `FTP curves published for ${fmtDateShort(p.as_of)}`, detail: `${p.curves.map((c) => c.ccy).join(', ')}; every desk sees them now.` });
      setFile(null);
      setPreview(null);
      setNote('');
    },
    onError: (e) => setError(e instanceof ApiError ? e.message : 'publishing failed; nothing was changed'),
  });

  const take = (f: File | undefined | null) => {
    if (!f) return;
    if (f.size > 2 * 1024 * 1024) {
      setError('the workbook is larger than 2 MB');
      return;
    }
    setFile(f);
    setPreview(null);
    read.mutate(f);
  };

  const cur = ftp.data?.current ?? null;
  // What publishing for the chosen day would replace.
  const onDay = useFtp(asOf || undefined);
  const replacing = onDay.data?.current && onDay.data.current.as_of === asOf ? onDay.data.current : null;
  const blocked = !preview || preview.errors.length > 0 || preview.curves.length === 0;
  return (
    <div className="alm-layout single">
      <section className="alm-results" data-ready="1" data-testid="ftp-publish">
        <div className="page-title">
          <h1 className="t-title">Publish FTP curves</h1>
          {cur && (
            <span className="t-small">
              in force: {fmtDateShort(cur.as_of)}, {cur.curves.map((c) => c.ccy).join(', ')}
            </span>
          )}
          <span className="grow" />
          <Link to="/ftp" className="btn btn-ghost btn-sm">
            See the curves in force
          </Link>
          <button type="button" className="btn btn-sm" onClick={() => endpoints.ftpTemplate().catch((e: Error) => setError(e.message))}>
            Download the template
          </button>
        </div>
        <p className="t-secondary prose">
          Drop ALM's workbook below. Either layout is read: one sheet with a <b>Tenor</b> column and a column per currency (BHD, USD, ...), or one sheet per
          currency named by its code. A cell labelled <b>As of</b> sets the date. The template is the curves in force, ready to edit.
        </p>

        <div
          className={`dropzone ${over ? 'over' : ''}`}
          data-testid="ftp-drop"
          role="button"
          tabIndex={0}
          onClick={() => input.current?.click()}
          onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && input.current?.click()}
          onDragOver={(e) => {
            e.preventDefault();
            setOver(true);
          }}
          onDragLeave={() => setOver(false)}
          onDrop={(e) => {
            e.preventDefault();
            setOver(false);
            take(e.dataTransfer.files?.[0]);
          }}
        >
          <input
            ref={input}
            type="file"
            accept=".xlsx,.xlsm,.xls,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.ms-excel"
            hidden
            aria-label="FTP workbook"
            onChange={(e) => {
              take(e.target.files?.[0]);
              e.target.value = '';
            }}
          />
          {read.isPending ? (
            <span className="t-medium">Reading {file?.name}…</span>
          ) : file ? (
            <span>
              <span className="t-medium">{file.name}</span> <span className="t-small">· drop another file to replace it</span>
            </span>
          ) : (
            <span>
              <span className="t-medium">Drop the FTP workbook here</span> <span className="t-small">or click to choose it (.xlsx, up to 2 MB)</span>
            </span>
          )}
        </div>

        {error && (
          <div className="notice bad" role="alert" style={{ marginTop: 8 }}>
            {error}
          </div>
        )}

        {preview && (
          <div className="ftp-preview" data-testid="ftp-preview">
            <div className="row" style={{ gap: 8, marginTop: 12 }}>
              <span className="t-heading">What the workbook holds</span>
              <span className="t-small">
                {preview.curves.length} {preview.curves.length === 1 ? 'curve' : 'curves'}
                {preview.layout ? `, read as ${preview.layout}` : ''}
                {preview.sheet_date ? `, dated ${fmtDateLong(preview.sheet_date)} in the workbook` : ', no date in the workbook'}
              </span>
              {preview.errors.length === 0 ? <Badge tone="good">ready to publish</Badge> : <Badge tone="bad">{preview.errors.length} to fix first</Badge>}
            </div>

            {preview.errors.length > 0 && (
              <ul className="issues bad" data-testid="ftp-errors">
                {preview.errors.map((e, i) => (
                  <li key={i}>
                    {e.cell && (
                      <span className="t-medium">
                        {e.sheet} {e.cell}
                      </span>
                    )}{' '}
                    {e.message}
                  </li>
                ))}
              </ul>
            )}
            {preview.fractions && <div className="notice">The rates were written as fractions (0.0525) and are read as percent (5.25%).</div>}
            {preview.warnings.length > 0 && (
              <ul className="issues warn">
                {preview.warnings.map((e, i) => (
                  <li key={i}>
                    {e.cell && (
                      <span className="t-medium">
                        {e.sheet} {e.cell}
                      </span>
                    )}{' '}
                    {e.message}
                  </li>
                ))}
              </ul>
            )}
            {preview.changes.length > 0 && (
              <>
                <div className="t-label" style={{ marginTop: 8 }}>
                  Against the curves of {preview.compared_with ? fmtDateShort(preview.compared_with) : 'the day before'}
                </div>
                <ul className="issues" data-testid="ftp-changes">
                  {preview.changes.map((c, i) => (
                    <li key={i}>{describe(c)}</li>
                  ))}
                </ul>
              </>
            )}

            {preview.curves.length > 0 && (
              <div className="panel-row two" style={{ marginTop: 10 }}>
                <div className="panel">
                  <FtpTable curves={preview.curves} previous={preview.previous.length ? preview.previous : null} />
                </div>
                <div className="panel">
                  <FtpCurvesChart curves={preview.curves} />
                </div>
              </div>
            )}

            <div className="save-box" style={{ marginTop: 12 }}>
              <div className="row" style={{ gap: 10, alignItems: 'flex-end', flexWrap: 'wrap' }}>
                <label className="field">
                  <span className="t-label">For</span>
                  <input className="input" type="date" value={asOf} onChange={(e) => setAsOf(e.target.value)} aria-label="Publication date" />
                </label>
                <label className="field grow">
                  <span className="t-label">Note</span>
                  <input className="input" placeholder="e.g. ALCO-approved curves for 5 November" value={note} onChange={(e) => setNote(e.target.value)} aria-label="Publication note" />
                </label>
                <button type="button" className="btn btn-primary" disabled={blocked || !note.trim() || !asOf || publish.isPending} onClick={() => publish.mutate()}>
                  {publish.isPending ? 'Publishing…' : 'Publish to every desk'}
                </button>
              </div>
              <div className="t-small prose" style={{ marginTop: 6 }}>
                {blocked ? (
                  'Fix the cells listed above in the workbook and drop it again.'
                ) : replacing ? (
                  <span data-testid="ftp-replaces">
                    Publishing replaces the curves for {fmtDateLong(asOf)} that <b>{replacing.published_by ?? 'the application'}</b> published {fmtLocal(replacing.published_at)}
                    {replacing.file_name ? ` from ${replacing.file_name}` : ''}. They stay in the upload log, marked replaced by you.
                  </span>
                ) : (
                  `Nothing is published for ${asOf ? fmtDateLong(asOf) : 'the date above'} yet; every desk sees these curves the moment you publish.`
                )}
              </div>
            </div>
          </div>
        )}

        <Section title="Upload log" meta="every workbook dropped here, by whom, and what replaced it; workbooks never published show only to ALM" />
        <FtpUploadLog testId="ftp-publish-log" />
      </section>
    </div>
  );
}
