void *malloc(int n);
void keep(void *p);
void skip(void);
void other(void);
void use1(int *p);
void run(int c) {
    int x = 0;
    int *w = &x;
    int ***m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = 0;
    if (c) {
        skip();
    } else {
        keep(m);
    }
    *m = &w;
    other();
    use1(w);
}
