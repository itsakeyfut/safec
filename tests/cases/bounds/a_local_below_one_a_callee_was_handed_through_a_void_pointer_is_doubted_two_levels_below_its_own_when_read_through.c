void put(void *p);
void use1(int *p);
void run(void) {
    int ***w = 0;
    int ****v = &w;
    put(&v);
    if (w != 0) {
        if (*w != 0) {
            int *r = **w;
            use1(r);
        }
    }
}
