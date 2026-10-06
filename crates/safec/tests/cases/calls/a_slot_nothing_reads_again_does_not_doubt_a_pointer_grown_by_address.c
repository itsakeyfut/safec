void *malloc(int n);
void grow(int **pp);
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
    grow(&a);
    return use2(&a) + (h != 0);
}
