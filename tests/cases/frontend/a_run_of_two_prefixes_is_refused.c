const char *utf8_and_wide = u8"a" L"b";
const void *two_wide = L"a" u"b";
const void *one_prefix = "a" L"b" "c";
const void *wide_and_utf8 = L"a" u8"b";
const void *after_an_innocent_line =
    "an innocent first line "
    u"x" L"y";
