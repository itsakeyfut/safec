void *malloc(int n);
void free(void *p);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int ***k = malloc(8);
    if (k == 0) {
        return 0;
    }
    free(a);
    *k = &a;
    return use2(*k);
}
