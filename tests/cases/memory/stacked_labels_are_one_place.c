void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    {
        int *q = p;
        if (c) goto A;
        {
            int *r = q;
            if (c > 1) goto B;
            free(r);
        }
    }
A:
B:
    ;
    {
        int *s = p;
        if (c > 2) goto A;
        free(s);
    }
    return 0;
}
