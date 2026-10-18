// Package inspector: the OPC parts and relationships of the open file.

import { useMemo, useState } from "react";

import type { DocInfo, OpenResult, PackageInfo, PartInfo } from "../engine/protocol";

export const fmtBytes = (n: number) =>
  n < 1024 ? `${n} B` : n < 1 << 20 ? `${(n / 1024).toFixed(1)} KiB` : `${(n / (1 << 20)).toFixed(2)} MiB`;

const shortType = (t: string) => t.slice(t.lastIndexOf("/") + 1);

export function PackageView({ open, pkg, info }: { open: OpenResult; pkg: PackageInfo; info: DocInfo }) {
  const [filter, setFilter] = useState("");
  const parts = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? pkg.parts.filter((p) => p.name.toLowerCase().includes(q) || (p.contentType ?? "").includes(q)) : pkg.parts;
  }, [pkg, filter]);

  return (
    <div className="content">
      <section className="summary">
        <h1>{open.fileName}</h1>
        <dl>
          <dt>Kind</dt>
          <dd>{pkg.kind}</dd>
          <dt>Main part</dt>
          <dd>{pkg.mainPart ?? "none"}</dd>
          <dt>Pages</dt>
          <dd>{open.pageCount}</dd>
          <dt>Parts</dt>
          <dd>
            {pkg.parts.length} ({fmtBytes(pkg.totalSize)} uncompressed)
          </dd>
          <dt>Relationships</dt>
          <dd>{pkg.relationships.length}</dd>
          <dt>CRDT snapshot</dt>
          <dd>{fmtBytes(info.snapshotBytes)}</dd>
        </dl>
        {pkg.warnings.length > 0 && (
          <ul className="warnings">
            {pkg.warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <div className="section-head">
          <h2>Parts</h2>
          <input
            type="search"
            placeholder="Filter parts"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            aria-label="Filter parts"
          />
        </div>
        <PartTable parts={parts} />
      </section>

      <section>
        <h2>Relationships</h2>
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Source</th>
                <th>Id</th>
                <th>Type</th>
                <th>Target</th>
              </tr>
            </thead>
            <tbody>
              {pkg.relationships.map((r) => (
                <tr key={`${r.source}#${r.id}`}>
                  <td className="mono">{r.source}</td>
                  <td className="mono">{r.id}</td>
                  <td title={r.relType}>{shortType(r.relType)}</td>
                  <td className={`mono${!r.external && !r.targetExists ? " missing" : ""}`}>
                    {r.external ? `${r.target} (external)` : (r.targetPart ?? r.target)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

function PartTable({ parts }: { parts: PartInfo[] }) {
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Part</th>
            <th>Content type</th>
            <th className="num">Size</th>
            <th className="num">Stored</th>
          </tr>
        </thead>
        <tbody>
          {parts.map((p) => (
            <tr key={p.name}>
              <td className="mono">{p.name}</td>
              <td className="ct">{p.contentType ?? "unknown"}</td>
              <td className="num">{fmtBytes(p.size)}</td>
              <td className="num">{fmtBytes(p.compressedSize)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
