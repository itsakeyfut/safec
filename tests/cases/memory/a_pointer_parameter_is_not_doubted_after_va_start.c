void *malloc(unsigned long long);
void free(void *);
struct H {
    int tag;
    __builtin_va_list ap;
};

int first(const char *fmt, ...) {
    if (!fmt)
        return 0;
    __builtin_va_list a;
    __builtin_va_start(a, fmt);
    int x = __builtin_va_arg(a, int);
    __builtin_va_end(a);
    return *fmt + x;
}

int a_list_in_a_heap_block(int n, ...) {
    struct H *h = malloc(sizeof(struct H));
    if (!h)
        return 0;
    h->tag = n;
    __builtin_va_start(h->ap, n);
    __builtin_va_end(h->ap);
    free(h);
    return n;
}
