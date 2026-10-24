void *malloc(int n);
void free(void *p);

int use2(int **pp) {
    int *q;
    if (pp == 0) {
        return 0;
    }
    q = *pp;
    if (q == 0) {
        return 0;
    }
    return *q;
}

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    free(a);
    return use2(&a);
}
