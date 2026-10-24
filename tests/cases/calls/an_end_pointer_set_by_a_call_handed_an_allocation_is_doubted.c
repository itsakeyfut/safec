void *malloc(int n);
void free(void *p);
int parse(char *s, char **end);
int f(void) {
    char *s = malloc(16); if (!s) return 0; *s = 0;
    char *end = 0;
    int v = parse(s, &end);
    if (!end) return v;
    return v + *end;
}
