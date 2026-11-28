static const char *p = "a";
static const int k = 1;
static const int *kp = &k;
static char c;
static int x;

char **one_level_down = (char **)&p;
void *through_void = &p;
struct Holder {
    char **pp;
} member = {(char **)&p};
char **table[2] = {0, (char **)&p};
int **an_integer_chain = (int **)&kp;
char ***a_compound_literal = &(char **){(char **)&p};

int *narrower = (int *)&c;

const char *const *every_level_const = &p;
static const char *names[2] = {"a", "b"};
int *const a_const_static = &x;

struct FirstConst {
    const char *s;
    int a;
};
static struct FirstConst first_const = {"x", 1};
char **a_member_at_offset_zero = (char **)&first_const.s;

union Pun {
    int i;
    const char *s;
};
static union Pun pun = {.s = "x"};
char **a_union_member = (char **)&pun.s;

struct Narrow {
    char c;
    int x;
};
static struct Narrow narrow;
int *a_narrow_member = (int *)&narrow.c;

static int four[2];
int *past_its_end = &four[2];
int *moved_past = four + 5;
