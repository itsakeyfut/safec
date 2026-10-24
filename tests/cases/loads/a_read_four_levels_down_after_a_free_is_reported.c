void *malloc(int n);
void free(void *p);

int main(void) {
    int ****t4 = malloc(8);
    if (t4 == 0) {
        return 0;
    }
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    *t2 = p;
    *t3 = t2;
    *t4 = t3;
    free(p);
    return ****t4;
}
