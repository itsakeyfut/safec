int called(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int (*f)(void) = __builtin_va_arg(a, int (*)(void));
    __builtin_va_end(a);
    if (!f)
        return 0;
    return f();
}
