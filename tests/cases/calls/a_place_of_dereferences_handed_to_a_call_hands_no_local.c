void *malloc(int n);
void drop(int *p);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **k = &a;
    drop(*k);
    return use2(&a);
}
