void *malloc(int n);
void g2(int *a, int **pp);
void use2(int **pp);
int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    g2(a, &a);
    use2(&a);
    return 0;
}
