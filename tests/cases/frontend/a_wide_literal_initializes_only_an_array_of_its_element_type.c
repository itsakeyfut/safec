int not_wchar[] = L"ab";
char not_wide[] = L"x";
unsigned short not_char[] = "x";
unsigned short is_wchar[] = L"ok";
int more_than_one = u'ab';
int two_elements_here = L'😀';
int past_wchar = sizeof(L"\xffffffff");
