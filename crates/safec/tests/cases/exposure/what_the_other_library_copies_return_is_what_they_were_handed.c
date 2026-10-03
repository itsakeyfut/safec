void *malloc(int n);
void free(void *p);
void *memmove(void *d, void *s, int n);
char *strncpy(char *d, char *s, int n);
char *strcat(char *d, char *s);
char *strncat(char *d, char *s, int n);

int main(void) {
    char *s = malloc(4);
    if (s == 0) {
        return 0;
    }
    *s = 0;
    char *a = memmove(malloc(4), s, 1);
    char *b = strncpy(malloc(4), s, 1);
    char *c = strcat(malloc(4), s);
    char *d = strncat(malloc(4), s, 1);
    free(a);
    free(b);
    free(c);
    free(d);
    free(s);
    return 0;
}
