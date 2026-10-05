void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    release_ref(&b);
    int *c = a;
    return use2(&c);
}
