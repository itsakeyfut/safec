void g(void);

int f(void *p) {
    if (p) {
        p ? *p : g();
    }
    return 0;
}
