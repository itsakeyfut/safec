void *malloc(int n);
char *strncpy(char *d, char *s, int n);

int f(char *s) {
    char *d = malloc(4);
    if (d == 0) {
        return 1;
    }
    strncpy(d, s, 4);
    return 0;
}
