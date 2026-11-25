int first(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int x = __builtin_va_arg(a, int);
    long long y = __builtin_va_arg(a, long long);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    return x + (int)y + (p != 0);
}
