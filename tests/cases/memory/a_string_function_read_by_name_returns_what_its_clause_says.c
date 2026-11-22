void free(void *p);
char *strdup(const char *s);
char *strndup(const char *s, unsigned long long n);
char *strpbrk(const char *s, const char *accept);
void a_copy_of_a_literal_is_written(void) {
    char *s = strdup("abc");
    if (s) {
        s[0] = 'x';
        free(s);
    }
}
void a_bounded_copy_of_a_literal_is_written(void) {
    char *s = strndup("abc", 2);
    if (s) {
        s[0] = 'x';
        free(s);
    }
}
void a_buffer_searched_with_a_literal_is_written(char *buf) {
    if (!buf) {
        return;
    }
    char *p = strpbrk(buf, " ");
    if (p) {
        *p = 0;
    }
}
void a_literal_searched_is_written(void) {
    char *p = strpbrk("a b", " ");
    if (p) {
        *p = 0;
    }
}
