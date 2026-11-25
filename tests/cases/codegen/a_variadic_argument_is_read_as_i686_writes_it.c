struct with_a_list {
    char c;
    __builtin_va_list list;
};

typedef __builtin_va_list list_t;

list_t global;

long long read(int n, ...) {
    __builtin_va_list a;
    __builtin_va_list b;
    __builtin_va_start(a, n);
    int x = __builtin_va_arg(a, int);
    __builtin_va_copy(b, a);
    long long y = __builtin_va_arg(b, long long);
    int *p = __builtin_va_arg(b, int *);
    __builtin_va_end(b);
    __builtin_va_end(a);
    return x + y + (p != 0) + sizeof(__builtin_va_list) + sizeof(struct with_a_list);
}
