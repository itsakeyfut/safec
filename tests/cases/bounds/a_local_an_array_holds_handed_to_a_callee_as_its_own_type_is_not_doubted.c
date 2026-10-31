void put(int **(*p)[1]);
void use1(int *p);
void run(void) {
    int *w = 0;
    int **a[1];
    a[0] = &w;
    put(&a);
    use1(w);
}
