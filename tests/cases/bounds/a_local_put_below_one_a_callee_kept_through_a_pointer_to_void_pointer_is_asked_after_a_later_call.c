void keep(void **p);
void other(void);
void use1(int *p);
void run(void) {
    int x = 0;
    int *w = &x;
    void *v = 0;
    keep(&v);
    v = &w;
    other();
    use1(w);
}
