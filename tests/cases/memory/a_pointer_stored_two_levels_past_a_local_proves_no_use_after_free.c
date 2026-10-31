void *malloc(int n);
void free(void *p);
int main(void) {
    int *z = 0;
    int *q = malloc(4);
    int **pp = &z;
    pp = pp + 7;
    int ***ppp = &pp;
    **ppp = q;
    free(z);
    return *q;
}
