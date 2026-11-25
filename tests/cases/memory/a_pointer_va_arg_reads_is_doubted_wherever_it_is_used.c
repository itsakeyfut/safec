int dereferenced(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    if (!p)
        return 0;
    return *p;
}
int null_tested_nowhere(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    return *p;
}
int integers_are_not_doubted(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int x = __builtin_va_arg(a, int);
    __builtin_va_end(a);
    return x + n;
}
