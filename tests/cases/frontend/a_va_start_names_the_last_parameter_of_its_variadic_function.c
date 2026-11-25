int fixed(int n) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    __builtin_va_end(a);
    return 0;
}
int not_the_last(int n, int m, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    __builtin_va_end(a);
    return m;
}
int not_a_list(int n, ...) {
    int *p = 0;
    __builtin_va_start(p, n);
    return 0;
}
int copied_by_assignment(int n, ...) {
    __builtin_va_list a;
    __builtin_va_list b;
    __builtin_va_start(a, n);
    b = a;
    __builtin_va_end(a);
    return 0;
}
