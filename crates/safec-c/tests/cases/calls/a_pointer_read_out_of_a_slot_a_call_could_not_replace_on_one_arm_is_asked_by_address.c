void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);

int f(int n) {
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
    int *c = 0;
    if (n) {
        c = *h;
    }
    return use2(&c);
}
