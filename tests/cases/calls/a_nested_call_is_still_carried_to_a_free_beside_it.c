void *malloc(int n);
void free(void *p);
char *strcpy(char *d, char *s);
int strlen(char *s);

int main(void) {
    char *s = malloc(8);
    char *t = malloc(8);
    if (s == 0) {
        return 0;
    }
    if (t == 0) {
        return 0;
    }
    t[0] = 0;
    return strlen(strcpy(s, t)) + (free(s), 0);
}
