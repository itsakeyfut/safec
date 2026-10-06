void *malloc(int n);
void release_ref(int **pp);
void release_buf(char **pp);
void next_token(char **pp);
int use2(int **pp);
void log_line(void);

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
    int *c = *h;
    return use2(&c);
}
