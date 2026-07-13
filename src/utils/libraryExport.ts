import {File, Paths} from 'expo-file-system';
import {Platform} from 'react-native';
import type {Book} from '../../modules/expo-rust-bridge';
import {createLibraryExportImage} from '../../modules/expo-rust-bridge';

export type LibraryExportFormat = 'csv' | 'json' | 'xlsx' | 'png' | 'txt' | 'goodreads';
export type LibraryExportSortField = 'title' | 'length';
export type LibraryExportDirection = 'asc' | 'desc';

export interface LibraryExportOptions {
    formats: LibraryExportFormat[];
    sortField: LibraryExportSortField;
    sortDirection: LibraryExportDirection;
    groupByAuthor: boolean;
    groupBySeries: boolean;
}

export interface LibraryExportFile {
    format: LibraryExportFormat;
    name: string;
    uri: string;
}

interface WritableExportFile {
    uri: string;
    write(content: string | Uint8Array, options: { encoding?: 'utf8' | 'base64' }): void;
}

interface WritableExportDirectory {
    createFile(name: string, mimeType: string | null): WritableExportFile;
}

interface ExportBook {
    productId: string;
    title: string;
    subtitle: string;
    authors: string;
    narrators: string;
    series: string;
    seriesSequence: number | null;
    length: string;
    lengthSeconds: number;
    releaseDate: string;
    purchaseDate: string;
    language: string;
    publisher: string;
    source: string;
    filePath: string;
    coverUrl: string;
    group: string;
}

interface SheetCell {
    value: string | number | null;
    type?: 'string' | 'number';
}

interface ZipFile {
    path: string;
    data: Uint8Array;
}

const EXPORT_HEADERS = [
    'Group',
    'Title',
    'Subtitle',
    'Authors',
    'Narrators',
    'Series',
    'Series #',
    'Length',
    'Length Seconds',
    'Release Date',
    'Purchase Date',
    'Language',
    'Publisher',
    'Source',
    'Product ID',
    'File Path',
    'Cover URL',
];

export async function exportLibrary(
    books: Book[],
    directory: WritableExportDirectory,
    options: LibraryExportOptions
): Promise<LibraryExportFile[]> {
    const exportedAt = new Date();
    const baseName = `librisync-library-${formatTimestamp(exportedAt)}`;
    const exportBooks = prepareBooks(books, options);
    const files: LibraryExportFile[] = [];

    for (const format of options.formats) {
        if (format === 'csv') {
            const name = `${baseName}.csv`;
            const file = writeTextFile(directory, name, 'text/csv', buildCsv(exportBooks));
            files.push({format, name, uri: file.uri});
        } else if (format === 'goodreads') {
            const name = `${baseName}-goodreads.csv`;
            const file = writeTextFile(directory, name, 'text/csv', buildGoodreadsCsv(exportBooks));
            files.push({format, name, uri: file.uri});
        } else if (format === 'txt') {
            const name = `${baseName}.txt`;
            const file = writeTextFile(directory, name, 'text/plain', buildText(exportBooks, options, exportedAt));
            files.push({format, name, uri: file.uri});
        } else if (format === 'json') {
            const name = `${baseName}.json`;
            const payload = buildJson(exportBooks, options, exportedAt);
            const file = writeTextFile(directory, name, 'application/json', payload);
            files.push({format, name, uri: file.uri});
        } else if (format === 'xlsx') {
            const name = `${baseName}.xlsx`;
            const file = writeBinaryFile(
                directory,
                name,
                'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
                buildXlsx(exportBooks)
            );
            files.push({format, name, uri: file.uri});
        } else if (format === 'png') {
            const name = `${baseName}.png`;
            const file = await writePngFile(directory, name, exportBooks, options, exportedAt);
            files.push({format, name, uri: file.uri});
        }
    }

    return files;
}

export function buildLibraryExportText(
    books: Book[],
    options: LibraryExportOptions,
    exportedAt: Date = new Date()
): string {
    return buildText(prepareBooks(books, options), options, exportedAt);
}

function prepareBooks(books: Book[], options: LibraryExportOptions): ExportBook[] {
    return books
        .map(book => normalizeBook(book, options))
        .sort((left, right) => compareBooks(left, right, options));
}

function normalizeBook(book: Book, options: LibraryExportOptions): ExportBook {
    const authors = joinPeople(book.authors);
    const series = book.series_name || '';
    const group = buildGroupLabel(authors, series, options);

    return {
        productId: book.audible_product_id || '',
        title: book.title || '',
        subtitle: book.subtitle || '',
        authors,
        narrators: joinPeople(book.narrators),
        series,
        seriesSequence: book.series_sequence ?? null,
        length: formatDuration(book.duration_seconds || 0),
        lengthSeconds: book.duration_seconds || 0,
        releaseDate: book.release_date || '',
        purchaseDate: book.purchase_date || '',
        language: book.language || '',
        publisher: book.publisher || '',
        source: book.source || 'audible',
        filePath: book.file_path || '',
        coverUrl: getExportCoverUrl(book.cover_url),
        group,
    };
}

function compareBooks(left: ExportBook, right: ExportBook, options: LibraryExportOptions): number {
    const direction = options.sortDirection === 'asc' ? 1 : -1;
    const groupCompare = compareText(left.group, right.group);
    if (groupCompare !== 0) return groupCompare;

    if (options.sortField === 'length') {
        const lengthCompare = left.lengthSeconds - right.lengthSeconds;
        if (lengthCompare !== 0) return lengthCompare * direction;
    } else {
        const titleCompare = compareText(left.title, right.title);
        if (titleCompare !== 0) return titleCompare * direction;
    }

    return compareText(left.title, right.title);
}

function buildGroupLabel(authors: string, series: string, options: LibraryExportOptions): string {
    const parts: string[] = [];
    if (options.groupByAuthor) parts.push(authors || 'Unknown Author');
    if (options.groupBySeries) parts.push(series || 'Standalone');
    return parts.join(' / ');
}

function joinPeople(values?: string[]): string {
    return (values || []).filter(Boolean).join(', ');
}

function formatDuration(seconds: number): string {
    const safeSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
    const hours = Math.floor(safeSeconds / 3600);
    const minutes = Math.floor((safeSeconds % 3600) / 60);
    return `${hours}h ${minutes}m`;
}

function getExportCoverUrl(coverUrl?: string): string {
    if (!coverUrl) return '';
    return coverUrl.replace(/_SL\d+_/, '_SL160_');
}

function buildCsv(books: ExportBook[]): string {
    const rows = [EXPORT_HEADERS, ...books.map(bookToRow)];
    return `\uFEFF${rows.map(row => row.map(escapeCsvValue).join(',')).join('\n')}`;
}

// Goodreads import format; StoryGraph accepts the same CSV.
// Audiobooks have no ISBN, so Goodreads/StoryGraph match by title + author.
const GOODREADS_HEADERS = [
    'Title',
    'Author',
    'ISBN',
    'ISBN13',
    'My Rating',
    'Publisher',
    'Binding',
    'Year Published',
    'Date Added',
    'Bookshelves',
    'Owned Copies',
];

function buildGoodreadsCsv(books: ExportBook[]): string {
    const rows = [
        GOODREADS_HEADERS,
        ...books.map(book => [
            book.title,
            book.authors,
            '',
            '',
            '0',
            book.publisher,
            'Audiobook',
            book.releaseDate ? book.releaseDate.slice(0, 4) : '',
            book.purchaseDate ? book.purchaseDate.slice(0, 10) : '',
            '',
            '1',
        ]),
    ];
    return `\uFEFF${rows.map(row => row.map(escapeCsvValue).join(',')).join('\n')}`;
}

function buildText(
    books: ExportBook[],
    options: LibraryExportOptions,
    exportedAt: Date
): string {
    const lines: string[] = [
        'LibriSync Library Export',
        `Exported: ${exportedAt.toLocaleString()}`,
        `Audiobooks: ${books.length}`,
        `Sort: ${options.sortField === 'title' ? 'Name' : 'Length'} ${options.sortDirection === 'asc' ? 'ascending' : 'descending'}`,
        `Grouped by: ${getTextGroupingLabel(options)}`,
        '',
    ];

    const grouped = options.groupByAuthor || options.groupBySeries;
    let currentGroup = '';
    let indexInGroup = 0;

    books.forEach((book, index) => {
        if (grouped && book.group !== currentGroup) {
            if (currentGroup) lines.push('');
            currentGroup = book.group || 'Library';
            indexInGroup = 0;
            lines.push(`${currentGroup}`);
        }

        indexInGroup += 1;
        const number = grouped ? indexInGroup : index + 1;
        const indent = grouped ? '  ' : '';
        const detailIndent = grouped ? '     ' : '   ';

        lines.push(`${indent}${number}. ${book.title || 'Untitled'}`);
        lines.push(`${detailIndent}- Authors: ${book.authors || 'Unknown Author'}`);
        if (book.series) {
            lines.push(`${detailIndent}- Series: ${book.series}${book.seriesSequence ? ` #${book.seriesSequence}` : ''}`);
        }
        lines.push(`${detailIndent}- Length: ${book.length}`);
        if (book.narrators) lines.push(`${detailIndent}- Narrators: ${book.narrators}`);
        if (book.publisher) lines.push(`${detailIndent}- Publisher: ${book.publisher}`);
        if (book.releaseDate) lines.push(`${detailIndent}- Release Date: ${book.releaseDate}`);
        if (book.purchaseDate) lines.push(`${detailIndent}- Purchase Date: ${book.purchaseDate}`);
        lines.push(`${detailIndent}- Source: ${formatSource(book.source)}`);
        lines.push(`${detailIndent}- Product ID: ${book.productId}`);
        lines.push('');
    });

    return lines.join('\n').trimEnd();
}

function getTextGroupingLabel(options: LibraryExportOptions): string {
    const groups = [];
    if (options.groupByAuthor) groups.push('author');
    if (options.groupBySeries) groups.push('series');
    return groups.length > 0 ? groups.join(' and ') : 'none';
}

function formatSource(source: string): string {
    if (source === 'librivox') return 'LibriVox';
    if (source === 'audible') return 'Audible';
    return source || 'Unknown';
}

function bookToRow(book: ExportBook): (string | number | null)[] {
    return [
        book.group,
        book.title,
        book.subtitle,
        book.authors,
        book.narrators,
        book.series,
        book.seriesSequence,
        book.length,
        book.lengthSeconds,
        book.releaseDate,
        book.purchaseDate,
        book.language,
        book.publisher,
        book.source,
        book.productId,
        book.filePath,
        book.coverUrl,
    ];
}

function escapeCsvValue(value: string | number | null): string {
    if (value === null || value === undefined) return '';
    const text = String(value);
    if (/[",\n\r]/.test(text)) {
        return `"${text.replace(/"/g, '""')}"`;
    }
    return text;
}

function buildJson(
    books: ExportBook[],
    options: LibraryExportOptions,
    exportedAt: Date
): string {
    const payload = {
        exported_at: exportedAt.toISOString(),
        count: books.length,
        sort: {
            field: options.sortField,
            direction: options.sortDirection,
        },
        group_by: {
            author: options.groupByAuthor,
            series: options.groupBySeries,
        },
        books: books.map(book => ({
            group: book.group || null,
            product_id: book.productId,
            title: book.title,
            subtitle: book.subtitle || null,
            authors: splitList(book.authors),
            narrators: splitList(book.narrators),
            series: book.series || null,
            series_sequence: book.seriesSequence,
            length: book.length,
            length_seconds: book.lengthSeconds,
            release_date: book.releaseDate || null,
            purchase_date: book.purchaseDate || null,
            language: book.language || null,
            publisher: book.publisher || null,
            source: book.source,
            file_path: book.filePath || null,
            cover_url: book.coverUrl || null,
        })),
        groups: buildGroups(books),
    };

    return JSON.stringify(payload, null, 2);
}

function splitList(value: string): string[] {
    return value ? value.split(', ').filter(Boolean) : [];
}

function buildGroups(books: ExportBook[]) {
    const groups = new Map<string, ExportBook[]>();
    books.forEach(book => {
        const key = book.group || 'Library';
        const groupBooks = groups.get(key) || [];
        groupBooks.push(book);
        groups.set(key, groupBooks);
    });

    return Array.from(groups.entries()).map(([name, groupBooks]) => ({
        name,
        count: groupBooks.length,
        product_ids: groupBooks.map(book => book.productId),
    }));
}

function buildXlsx(books: ExportBook[]): Uint8Array {
    const rows: SheetCell[][] = [
        EXPORT_HEADERS.map(header => ({value: header})),
        ...books.map(book => bookToRow(book).map(value => ({
            value,
            type: typeof value === 'number' ? 'number' as const : 'string' as const,
        }))),
    ];

    const sheetXml = buildWorksheetXml(rows);
    const workbookXml = xmlFile(`\
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Library" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>`);

    return createZip([
        {
            path: '[Content_Types].xml',
            data: textToUtf8(xmlFile(`\
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>`)),
        },
        {
            path: '_rels/.rels',
            data: textToUtf8(xmlFile(`\
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>`)),
        },
        {
            path: 'xl/workbook.xml',
            data: textToUtf8(workbookXml),
        },
        {
            path: 'xl/_rels/workbook.xml.rels',
            data: textToUtf8(xmlFile(`\
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>`)),
        },
        {
            path: 'xl/styles.xml',
            data: textToUtf8(xmlFile(`\
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="2">
    <font><sz val="11"/><name val="Calibri"/></font>
    <font><b/><sz val="11"/><name val="Calibri"/></font>
  </fonts>
  <fills count="2">
    <fill><patternFill patternType="none"/></fill>
    <fill><patternFill patternType="gray125"/></fill>
  </fills>
  <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
  <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="2">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0"/>
  </cellXfs>
</styleSheet>`)),
        },
        {
            path: 'xl/worksheets/sheet1.xml',
            data: textToUtf8(sheetXml),
        },
    ]);
}

function buildWorksheetXml(rows: SheetCell[][]): string {
    const sheetRows = rows.map((row, rowIndex) => {
        const rowNumber = rowIndex + 1;
        const cells = row.map((cell, columnIndex) => {
            const reference = `${columnName(columnIndex + 1)}${rowNumber}`;
            const style = rowIndex === 0 ? ' s="1"' : '';
            if (cell.type === 'number' && typeof cell.value === 'number') {
                return `<c r="${reference}"${style}><v>${cell.value}</v></c>`;
            }
            return `<c r="${reference}" t="inlineStr"${style}><is><t>${escapeXml(String(cell.value ?? ''))}</t></is></c>`;
        }).join('');
        return `<row r="${rowNumber}">${cells}</row>`;
    }).join('');

    return xmlFile(`\
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <cols>
    <col min="1" max="1" width="28" customWidth="1"/>
    <col min="2" max="2" width="40" customWidth="1"/>
    <col min="3" max="6" width="24" customWidth="1"/>
    <col min="7" max="9" width="14" customWidth="1"/>
    <col min="10" max="17" width="22" customWidth="1"/>
  </cols>
  <sheetData>${sheetRows}</sheetData>
</worksheet>`);
}

function columnName(index: number): string {
    let name = '';
    let current = index;
    while (current > 0) {
        current--;
        name = String.fromCharCode(65 + (current % 26)) + name;
        current = Math.floor(current / 26);
    }
    return name;
}

async function writePngFile(
    directory: WritableExportDirectory,
    name: string,
    books: ExportBook[],
    options: LibraryExportOptions,
    exportedAt: Date
): Promise<WritableExportFile> {
    if (Platform.OS !== 'android') {
        throw new Error('PNG export is only available on Android in this build.');
    }

    const tempFile = new File(Paths.cache, name);
    if (tempFile.exists) tempFile.delete();

    await createLibraryExportImage(
        JSON.stringify(buildImageEntries(books, options, exportedAt)),
        tempFile.uri
    );

    const outputFile = directory.createFile(name, 'image/png');
    outputFile.write(await tempFile.bytes(), {});
    tempFile.delete();

    return outputFile;
}

function buildImageEntries(
    books: ExportBook[],
    options: LibraryExportOptions,
    exportedAt: Date
) {
    const entries: Array<Record<string, string | number>> = [
        {
            type: 'header',
            title: 'Library Export',
            subtitle: `${books.length} audiobooks - ${exportedAt.toLocaleString()}`,
        },
    ];

    let lastGroup = '';
    books.forEach(book => {
        if ((options.groupByAuthor || options.groupBySeries) && book.group !== lastGroup) {
            entries.push({
                type: 'group',
                title: book.group || 'Library',
            });
            lastGroup = book.group;
        }

        entries.push({
            type: 'book',
            title: book.title,
            authors: book.authors || 'Unknown Author',
            series: book.series ? `${book.series}${book.seriesSequence ? ` #${book.seriesSequence}` : ''}` : '',
            length: book.length,
            cover_url: book.coverUrl,
        });
    });

    return entries;
}

function writeTextFile(directory: WritableExportDirectory, name: string, mimeType: string, contents: string): WritableExportFile {
    const file = directory.createFile(name, mimeType);
    file.write(contents, {encoding: 'utf8'});
    return file;
}

function writeBinaryFile(directory: WritableExportDirectory, name: string, mimeType: string, contents: Uint8Array): WritableExportFile {
    const file = directory.createFile(name, mimeType);
    file.write(contents, {});
    return file;
}

function formatTimestamp(date: Date): string {
    const pad = (value: number) => String(value).padStart(2, '0');
    const year = date.getFullYear();
    const month = pad(date.getMonth() + 1);
    const day = pad(date.getDate());
    const hours = pad(date.getHours());
    const minutes = pad(date.getMinutes());
    const seconds = pad(date.getSeconds());
    return `${year}${month}${day}-${hours}${minutes}${seconds}`;
}

function xmlFile(contents: string): string {
    return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n${contents}`;
}

function escapeXml(value: string): string {
    return value
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&apos;');
}

function compareText(left: string, right: string): number {
    return left.localeCompare(right, undefined, {sensitivity: 'base'});
}

function textToUtf8(text: string): Uint8Array {
    const bytes: number[] = [];

    for (let index = 0; index < text.length; index++) {
        let codePoint = text.charCodeAt(index);

        if (codePoint >= 0xD800 && codePoint <= 0xDBFF && index + 1 < text.length) {
            const next = text.charCodeAt(index + 1);
            if (next >= 0xDC00 && next <= 0xDFFF) {
                codePoint = 0x10000 + ((codePoint - 0xD800) << 10) + (next - 0xDC00);
                index++;
            }
        }

        if (codePoint <= 0x7F) {
            bytes.push(codePoint);
        } else if (codePoint <= 0x7FF) {
            bytes.push(0xC0 | (codePoint >> 6));
            bytes.push(0x80 | (codePoint & 0x3F));
        } else if (codePoint <= 0xFFFF) {
            bytes.push(0xE0 | (codePoint >> 12));
            bytes.push(0x80 | ((codePoint >> 6) & 0x3F));
            bytes.push(0x80 | (codePoint & 0x3F));
        } else {
            bytes.push(0xF0 | (codePoint >> 18));
            bytes.push(0x80 | ((codePoint >> 12) & 0x3F));
            bytes.push(0x80 | ((codePoint >> 6) & 0x3F));
            bytes.push(0x80 | (codePoint & 0x3F));
        }
    }

    return new Uint8Array(bytes);
}

function createZip(files: ZipFile[]): Uint8Array {
    const chunks: Uint8Array[] = [];
    const centralDirectory: Uint8Array[] = [];
    let offset = 0;
    const {time, date} = getDosDateTime(new Date());

    files.forEach(file => {
        const nameBytes = textToUtf8(file.path);
        const crc = crc32(file.data);
        const localHeader = new Uint8Array(30 + nameBytes.length);

        writeUint32(localHeader, 0, 0x04034b50);
        writeUint16(localHeader, 4, 20);
        writeUint16(localHeader, 6, 0x0800);
        writeUint16(localHeader, 8, 0);
        writeUint16(localHeader, 10, time);
        writeUint16(localHeader, 12, date);
        writeUint32(localHeader, 14, crc);
        writeUint32(localHeader, 18, file.data.length);
        writeUint32(localHeader, 22, file.data.length);
        writeUint16(localHeader, 26, nameBytes.length);
        writeUint16(localHeader, 28, 0);
        localHeader.set(nameBytes, 30);

        chunks.push(localHeader, file.data);

        const centralHeader = new Uint8Array(46 + nameBytes.length);
        writeUint32(centralHeader, 0, 0x02014b50);
        writeUint16(centralHeader, 4, 20);
        writeUint16(centralHeader, 6, 20);
        writeUint16(centralHeader, 8, 0x0800);
        writeUint16(centralHeader, 10, 0);
        writeUint16(centralHeader, 12, time);
        writeUint16(centralHeader, 14, date);
        writeUint32(centralHeader, 16, crc);
        writeUint32(centralHeader, 20, file.data.length);
        writeUint32(centralHeader, 24, file.data.length);
        writeUint16(centralHeader, 28, nameBytes.length);
        writeUint16(centralHeader, 30, 0);
        writeUint16(centralHeader, 32, 0);
        writeUint16(centralHeader, 34, 0);
        writeUint16(centralHeader, 36, 0);
        writeUint32(centralHeader, 38, 0);
        writeUint32(centralHeader, 42, offset);
        centralHeader.set(nameBytes, 46);
        centralDirectory.push(centralHeader);

        offset += localHeader.length + file.data.length;
    });

    const centralDirectoryStart = offset;
    centralDirectory.forEach(header => {
        chunks.push(header);
        offset += header.length;
    });

    const endRecord = new Uint8Array(22);
    writeUint32(endRecord, 0, 0x06054b50);
    writeUint16(endRecord, 4, 0);
    writeUint16(endRecord, 6, 0);
    writeUint16(endRecord, 8, files.length);
    writeUint16(endRecord, 10, files.length);
    writeUint32(endRecord, 12, offset - centralDirectoryStart);
    writeUint32(endRecord, 16, centralDirectoryStart);
    writeUint16(endRecord, 20, 0);
    chunks.push(endRecord);

    return concatBytes(chunks);
}

function concatBytes(chunks: Uint8Array[]): Uint8Array {
    const totalLength = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const output = new Uint8Array(totalLength);
    let offset = 0;
    chunks.forEach(chunk => {
        output.set(chunk, offset);
        offset += chunk.length;
    });
    return output;
}

function getDosDateTime(date: Date): { time: number; date: number } {
    const time = (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2);
    const dosDate = ((date.getFullYear() - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
    return {time, date: dosDate};
}

function writeUint16(buffer: Uint8Array, offset: number, value: number): void {
    buffer[offset] = value & 0xff;
    buffer[offset + 1] = (value >>> 8) & 0xff;
}

function writeUint32(buffer: Uint8Array, offset: number, value: number): void {
    buffer[offset] = value & 0xff;
    buffer[offset + 1] = (value >>> 8) & 0xff;
    buffer[offset + 2] = (value >>> 16) & 0xff;
    buffer[offset + 3] = (value >>> 24) & 0xff;
}

const CRC_TABLE = (() => {
    const table: number[] = [];
    for (let n = 0; n < 256; n++) {
        let c = n;
        for (let k = 0; k < 8; k++) {
            c = (c & 1) ? (0xedb88320 ^ (c >>> 1)) : (c >>> 1);
        }
        table[n] = c >>> 0;
    }
    return table;
})();

function crc32(data: Uint8Array): number {
    let crc = 0xffffffff;
    for (let index = 0; index < data.length; index++) {
        crc = CRC_TABLE[(crc ^ data[index]) & 0xff] ^ (crc >>> 8);
    }
    return (crc ^ 0xffffffff) >>> 0;
}
