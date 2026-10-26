void use(int ***p);
void run(void **m) {
    void *v = m;
    int **x = v;
    use(&x);
}
