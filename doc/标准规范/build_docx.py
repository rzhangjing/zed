# -*- coding: utf-8 -*-
"""把《UI界面合规性验收规范.md》渲染为 DOCX。

只支持该文档实际用到的 Markdown 子集：
标题、段落、无序/有序列表、引用、围栏代码块、管道表格、行内 **粗体** 与 `代码`。
"""

import re
import sys
from pathlib import Path

from docx import Document
from docx.enum.section import WD_SECTION
from docx.enum.table import WD_TABLE_ALIGNMENT
from docx.enum.text import WD_ALIGN_PARAGRAPH, WD_LINE_SPACING
from docx.oxml import OxmlElement
from docx.oxml.ns import qn
from docx.shared import Cm, Pt, RGBColor

DEFAULT_SRC = Path(r"E:\Rust\标准规范\UI界面验收规范.md")
DEFAULT_DST = Path(r"E:\Rust\标准规范\UI界面验收规范.docx")
DEFAULT_FOOTER = "UI 界面验收规范（STD-UI-ACC-002）"
DEFAULT_TITLE = "UI 界面验收规范"
DEFAULT_VERSION = "v2.4"

_argv = sys.argv[1:]
SRC = Path(_argv[0]) if len(_argv) > 0 else DEFAULT_SRC
DST = Path(_argv[1]) if len(_argv) > 1 else DEFAULT_DST
FOOTER = _argv[2] if len(_argv) > 2 else DEFAULT_FOOTER
DOC_TITLE = _argv[3] if len(_argv) > 3 else DEFAULT_TITLE
DOC_VERSION = _argv[4] if len(_argv) > 4 else DEFAULT_VERSION

BODY_CN = "宋体"
BODY_EN = "Times New Roman"
HEAD_CN = "黑体"
CODE_EN = "Consolas"
ACCENT = RGBColor(0x1F, 0x3B, 0x63)
MUTED = RGBColor(0x59, 0x59, 0x59)


# ---------------------------------------------------------------- 基础样式工具
def set_run_font(run, cn=BODY_CN, en=BODY_EN, size=None, bold=None, color=None):
    run.font.name = en
    rpr = run._element.get_or_add_rPr()
    rfonts = rpr.find(qn("w:rFonts"))
    if rfonts is None:
        rfonts = OxmlElement("w:rFonts")
        rpr.append(rfonts)
    rfonts.set(qn("w:ascii"), en)
    rfonts.set(qn("w:hAnsi"), en)
    rfonts.set(qn("w:eastAsia"), cn)
    if size is not None:
        run.font.size = Pt(size)
    if bold is not None:
        run.font.bold = bold
    if color is not None:
        run.font.color.rgb = color


def shade(element, fill):
    shd = OxmlElement("w:shd")
    shd.set(qn("w:val"), "clear")
    shd.set(qn("w:color"), "auto")
    shd.set(qn("w:fill"), fill)
    element.append(shd)


def cell_shade(cell, fill):
    shade(cell._tc.get_or_add_tcPr(), fill)


def add_bottom_border(paragraph, color="1F3B63", size=8):
    ppr = paragraph._p.get_or_add_pPr()
    borders = OxmlElement("w:pBdr")
    bottom = OxmlElement("w:bottom")
    bottom.set(qn("w:val"), "single")
    bottom.set(qn("w:sz"), str(size))
    bottom.set(qn("w:space"), "1")
    bottom.set(qn("w:color"), color)
    borders.append(bottom)
    ppr.append(borders)


def add_field(paragraph, instr, size=9):
    run = paragraph.add_run()
    set_run_font(run, size=size, color=MUTED)
    start = OxmlElement("w:fldChar")
    start.set(qn("w:fldCharType"), "begin")
    text = OxmlElement("w:instrText")
    text.set(qn("xml:space"), "preserve")
    text.text = instr
    end = OxmlElement("w:fldChar")
    end.set(qn("w:fldCharType"), "end")
    run._r.append(start)
    run._r.append(text)
    run._r.append(end)


# ---------------------------------------------------------------- 行内格式
INLINE = re.compile(r"(\*\*.+?\*\*|`[^`]+`)")


def add_inline(paragraph, text, size=10.5, cn=BODY_CN, en=BODY_EN, base_bold=False, color=None):
    """把一行带行内标记的文本写入段落。"""
    for token in INLINE.split(text):
        if not token:
            continue
        if token.startswith("**") and token.endswith("**") and len(token) > 4:
            inner = token[2:-2]
            for sub in re.split(r"(`[^`]+`)", inner):
                if not sub:
                    continue
                if sub.startswith("`") and sub.endswith("`") and len(sub) > 2:
                    run = paragraph.add_run(sub[1:-1])
                    set_run_font(run, cn=BODY_CN, en=CODE_EN, size=size - 0.5,
                                 bold=True, color=RGBColor(0x0B, 0x4F, 0x6C))
                else:
                    run = paragraph.add_run(sub)
                    set_run_font(run, cn=cn, en=en, size=size, bold=True, color=color)
        elif token.startswith("`") and token.endswith("`") and len(token) > 2:
            run = paragraph.add_run(token[1:-1])
            set_run_font(run, cn=BODY_CN, en=CODE_EN, size=size - 0.5, color=RGBColor(0x0B, 0x4F, 0x6C))
        else:
            run = paragraph.add_run(token)
            set_run_font(run, cn=cn, en=en, size=size, bold=base_bold, color=color)


# ---------------------------------------------------------------- 文档骨架
def new_document():
    doc = Document()

    normal = doc.styles["Normal"]
    normal.font.name = BODY_EN
    normal.font.size = Pt(10.5)
    rpr = normal.element.get_or_add_rPr()
    rfonts = rpr.find(qn("w:rFonts"))
    if rfonts is None:
        rfonts = OxmlElement("w:rFonts")
        rpr.append(rfonts)
    rfonts.set(qn("w:eastAsia"), BODY_CN)
    normal.paragraph_format.line_spacing_rule = WD_LINE_SPACING.MULTIPLE
    normal.paragraph_format.line_spacing = 1.45
    normal.paragraph_format.space_after = Pt(4)

    for level, size in ((1, 15), (2, 13), (3, 11.5), (4, 10.5)):
        st = doc.styles[f"Heading {level}"]
        st.font.name = BODY_EN
        st.font.size = Pt(size)
        st.font.bold = True
        st.font.italic = False
        st.font.color.rgb = ACCENT
        srpr = st.element.get_or_add_rPr()
        srfonts = srpr.find(qn("w:rFonts"))
        if srfonts is None:
            srfonts = OxmlElement("w:rFonts")
            srpr.append(srfonts)
        srfonts.set(qn("w:eastAsia"), HEAD_CN)
        st.paragraph_format.space_before = Pt(10 if level <= 2 else 7)
        st.paragraph_format.space_after = Pt(4)
        st.paragraph_format.keep_with_next = True

    section = doc.sections[0]
    section.page_width = Cm(21.0)
    section.page_height = Cm(29.7)
    section.top_margin = Cm(2.3)
    section.bottom_margin = Cm(2.1)
    section.left_margin = Cm(2.4)
    section.right_margin = Cm(2.2)

    footer = section.footer
    fp = footer.paragraphs[0]
    fp.alignment = WD_ALIGN_PARAGRAPH.CENTER
    run = fp.add_run(f"{FOOTER}　·　第 ")
    set_run_font(run, size=9, color=MUTED)
    add_field(fp, "PAGE")
    run = fp.add_run(" 页")
    set_run_font(run, size=9, color=MUTED)

    return doc


# ---------------------------------------------------------------- 块渲染
def render_heading(doc, text, level, first_h1, page_break_before=False):
    if level == 1 and first_h1:
        p = doc.add_paragraph()
        p.paragraph_format.space_before = Pt(24)
        p.paragraph_format.space_after = Pt(10)
        p.alignment = WD_ALIGN_PARAGRAPH.CENTER
        add_inline(p, text, size=24, cn=HEAD_CN, en=HEAD_CN, base_bold=True, color=ACCENT)
        return
    style_level = level
    p = doc.add_paragraph(style=f"Heading {style_level}")
    if page_break_before:
        p.paragraph_format.page_break_before = True
    add_inline(p, text, size={1: 15, 2: 13, 3: 11.5, 4: 10.5}.get(style_level, 10.5),
               cn=HEAD_CN, en=HEAD_CN, base_bold=True, color=ACCENT)
    if style_level == 1:
        add_bottom_border(p)


def render_code_block(doc, lines):
    for idx, line in enumerate(lines):
        p = doc.add_paragraph()
        pf = p.paragraph_format
        pf.left_indent = Cm(0.6)
        pf.space_before = Pt(3 if idx == 0 else 0)
        pf.space_after = Pt(3 if idx < len(lines) - 1 else 9)
        pf.line_spacing = 1.0
        # 短代码块整块不跨页；长清单允许断开，否则会把整页留白
        if len(lines) <= 20 and idx < len(lines) - 1:
            pf.keep_with_next = True
        shade(p._p.get_or_add_pPr(), "F4F5F7")
        run = p.add_run(line if line else " ")
        set_run_font(run, cn=BODY_CN, en=CODE_EN, size=8)


def render_table(doc, rows):
    header, body = rows[0], rows[1:]
    ncol = len(header)
    widths = []
    for i in range(ncol):
        longest = max(len(str(r[i])) if i < len(r) else 0 for r in rows)
        # 阻尼：避免单个超长单元格把其余列压扁
        widths.append(max(2, min(longest, 30)) ** 0.7)
    total = sum(widths) or 1
    usable = 16.4
    col_w = [max(1.2, usable * w / total) for w in widths]
    scale = usable / sum(col_w)
    col_w = [w * scale for w in col_w]

    table = doc.add_table(rows=0, cols=ncol)
    table.style = "Table Grid"
    table.alignment = WD_TABLE_ALIGNMENT.CENTER
    table.autofit = False
    tbl_pr = table._tbl.tblPr
    layout = OxmlElement("w:tblLayout")
    layout.set(qn("w:type"), "fixed")
    tbl_pr.append(layout)

    def fill(cells, values, bold=False, fill_color=None):
        for idx, cell in enumerate(cells):
            text = values[idx] if idx < len(values) else ""
            cell.text = ""
            p = cell.paragraphs[0]
            p.paragraph_format.space_before = Pt(2)
            p.paragraph_format.space_after = Pt(2)
            p.paragraph_format.line_spacing = 1.15
            add_inline(p, text, size=8.5, base_bold=bold,
                       color=RGBColor(0x1F, 0x3B, 0x63) if bold else None)
            if fill_color:
                cell_shade(cell, fill_color)

    row = table.add_row()
    fill(row.cells, header, bold=True, fill_color="E8EDF5")
    table.rows[0]._tr.get_or_add_trPr().append(OxmlElement("w:tblHeader"))

    for values in body:
        row = table.add_row()
        fill(row.cells, values)

    for idx, width in enumerate(col_w):
        table.columns[idx].width = Cm(width)
    for row in table.rows:
        tr_pr = row._tr.get_or_add_trPr()
        tr_pr.append(OxmlElement("w:cantSplit"))
        for idx, cell in enumerate(row.cells):
            cell.width = Cm(col_w[idx])

    # 小表（≤10 行）整体不跨页，避免拆成零碎几行
    if len(table.rows) <= 10:
        for row in table.rows[:-1]:
            for cell in row.cells:
                for paragraph in cell.paragraphs:
                    paragraph.paragraph_format.keep_with_next = True

    spacer = doc.add_paragraph()
    spacer.paragraph_format.space_after = Pt(2)
    spacer.paragraph_format.line_spacing = 1.0
    for run in spacer.runs:
        set_run_font(run, size=2)


def split_row(line):
    raw = line.strip().strip("|")
    parts = [p.strip() for p in raw.split("|")]
    return parts


def is_separator(line):
    return bool(re.fullmatch(r"\|?[\s:\-|]+\|?", line.strip())) and "-" in line and "|" in line


def main():
    text = SRC.read_text(encoding="utf-8")
    text = re.sub(r"<!--.*?-->", "", text, flags=re.S)
    lines = text.split("\n")

    doc = new_document()
    first_h1 = True
    i = 0
    in_code = False
    code_buf = []
    table_buf = []
    page_break_pending = False

    def flush_table():
        nonlocal table_buf
        if table_buf:
            render_table(doc, table_buf)
            table_buf = []

    def flush_code():
        nonlocal code_buf
        if code_buf:
            render_code_block(doc, code_buf)
            code_buf = []

    while i < len(lines):
        line = lines[i]
        stripped = line.strip()

        if stripped.startswith("```"):
            flush_table()
            if in_code:
                flush_code()
                in_code = False
            else:
                in_code = True
            i += 1
            continue

        if in_code:
            code_buf.append(line.rstrip())
            i += 1
            continue

        if stripped.startswith("|") and stripped.count("|") >= 2:
            flush_code()
            if is_separator(stripped):
                i += 1
                continue
            table_buf.append(split_row(stripped))
            i += 1
            continue

        flush_table()

        if not stripped:
            i += 1
            continue

        if stripped == "---":
            i += 1
            continue

        if stripped.startswith("#"):
            m = re.match(r"(#{1,6})\s+(.*)", stripped)
            if m:
                level = len(m.group(1))
                heading_text = m.group(2).strip()
                if heading_text in ():
                    page_break_pending = True
                render_heading(doc, heading_text, level, first_h1,
                               page_break_before=page_break_pending)
                page_break_pending = False
                if level == 1:
                    first_h1 = False
                i += 1
                continue

        if stripped.startswith("> "):
            p = doc.add_paragraph()
            p.paragraph_format.left_indent = Cm(0.8)
            p.paragraph_format.space_before = Pt(4)
            p.paragraph_format.space_after = Pt(6)
            add_inline(p, stripped[2:], size=9.5, base_bold=False, color=MUTED)
            p.runs[0].font.italic = True
            i += 1
            continue

        m = re.match(r"^(\s*)- \[( |x)\] (.*)$", line)
        if m:
            p = doc.add_paragraph()
            p.paragraph_format.left_indent = Cm(0.9)
            p.paragraph_format.first_line_indent = Cm(-0.45)
            mark = "☑ " if m.group(2) == "x" else "☐ "
            run = p.add_run(mark)
            set_run_font(run, cn=BODY_CN, en=BODY_EN, size=10.5)
            add_inline(p, m.group(3))
            i += 1
            continue

        m = re.match(r"^(\s*)[-*]\s+(.*)$", line)
        if m:
            depth = len(m.group(1)) // 2
            p = doc.add_paragraph(style="List Bullet")
            p.paragraph_format.left_indent = Cm(0.75 + 0.5 * depth)
            p.paragraph_format.space_after = Pt(2)
            add_inline(p, m.group(2))
            i += 1
            continue

        m = re.match(r"^(\s*)(\d+)\.\s+(.*)$", line)
        if m:
            # 同一编号列表整段不跨页：向后跳过空行找下一项
            has_next = False
            for nxt in lines[i + 1:]:
                if re.match(r"^\s*\d+\.\s+", nxt):
                    has_next = True
                    break
                if nxt.strip() == "":
                    continue
                break
            p = doc.add_paragraph()
            p.paragraph_format.left_indent = Cm(0.9)
            p.paragraph_format.first_line_indent = Cm(-0.55)
            p.paragraph_format.space_after = Pt(2)
            if has_next:
                p.paragraph_format.keep_with_next = True
            add_inline(p, f"{m.group(2)}. {m.group(3)}")
            i += 1
            continue

        p = doc.add_paragraph()
        p.paragraph_format.first_line_indent = Cm(0)
        if stripped.endswith("：") or stripped.endswith(":"):
            p.paragraph_format.keep_with_next = True
        for nxt in lines[i + 1:]:
            if nxt.strip():
                if nxt.strip().startswith("|"):
                    p.paragraph_format.keep_with_next = True
                break
        add_inline(p, stripped)
        i += 1

    flush_table()
    flush_code()

    core = doc.core_properties
    core.title = DOC_TITLE
    core.subject = "功能完整性 · 安全性 · 鲁棒性 · 友好性"
    core.author = "STD-UI"
    core.category = "标准规范"
    core.comments = DOC_VERSION

    doc.save(DST)
    print(f"saved: {DST}")


if __name__ == "__main__":
    sys.exit(main())
