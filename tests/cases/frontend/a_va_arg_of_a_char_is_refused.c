int a_char(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    char c = __builtin_va_arg(a, char);
    __builtin_va_end(a);
    return c;
}
