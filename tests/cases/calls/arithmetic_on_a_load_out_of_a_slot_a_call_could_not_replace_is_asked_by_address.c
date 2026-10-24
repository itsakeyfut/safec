void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **h = malloc(8);
    if (h == 0) {
        return 0;
    }
    *h = a;
    int *b = a;
    release_ref(&b);
    int *e = *h + 1;
    return use2(&e);
}
