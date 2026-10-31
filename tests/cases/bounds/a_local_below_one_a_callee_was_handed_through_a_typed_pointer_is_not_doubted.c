void put(int ***p);
void use1(int *p);
void run(void) {
    int *w = 0;
    int **v = &w;
    put(&v);
    use1(w);
}
