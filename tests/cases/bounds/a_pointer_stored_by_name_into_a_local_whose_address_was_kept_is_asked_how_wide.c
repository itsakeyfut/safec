void *malloc(int n);
void keep(int **pp);
void use(void);
void f(void) {
    int *slot = 0;
    keep(&slot);
    void *v = malloc(1);
    slot = v;
    use();
}
