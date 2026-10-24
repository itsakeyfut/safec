void *malloc(int n);
void grow(int **pp);
int use2(int **pp);

int f(int c) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = malloc(4);
    if (b == 0) {
        return 0;
    }
    int **q = &a;
    if (c) {
        q = &b;
    }
    grow(q);
    return use2(&a) + (b != 0);
}
