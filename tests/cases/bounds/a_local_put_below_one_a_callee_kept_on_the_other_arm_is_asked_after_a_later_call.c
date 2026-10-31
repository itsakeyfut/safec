void keep(void *p);
void skip(void);
void other(void);
void use1(int *p);
void run(int c) {
    int x = 0;
    int *w = &x;
    int **v = 0;
    if (c) {
        skip();
    } else {
        keep(&v);
    }
    v = &w;
    other();
    use1(w);
}
