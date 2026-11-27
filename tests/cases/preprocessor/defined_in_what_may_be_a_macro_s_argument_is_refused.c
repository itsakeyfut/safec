#define X 1
#define F(a) (a+1)
#define G F
#define OPEN F(
#define CALL(m) m
#define NOT !
#define ALSO_OPEN OPEN
#define SELF SELF
#define F2(a) F(
#define VIA_CALL F2(0)

#if F(defined(X)) == 2
int a;
#endif

#if G (defined X) == 2
int b;
#endif

#if OPEN defined X) == 2
int c;
#endif

#if CALL(F)(defined X) == 2
int e;
#endif

#if ALSO_OPEN defined X) == 2
int h;
#endif

#if VIA_CALL defined X) == 2
int j;
#endif

#define D0 D1 D1
#define D1 D2 D2
#define D2 D3 D3
#define D3 D4 D4
#define D4 D5 D5
#define D5 D6 D6
#define D6 D7 D7
#define D7 D8 D8
#define D8 D9 D9
#define D9 D10 D10
#define D10 D11 D11
#define D11 D12 D12
#define D12 D13 D13
#define D13 D14 D14
#define D14 D15 D15
#define D15 D16 D16
#define D16 D17 D17
#define D17 D18 D18
#define D18 D19 D19
#define D19 D20 D20
#define D20 D21 D21
#define D21 D22 D22
#define D22 D23 D23
#define D23 D24 D24
#define D24 D25 D25
#define D25 D26 D26
#define D26 D27 D27
#define D27 D28 D28
#define D28 D29 D29
#define D29 D30 D30
#define D30 D31 D31
#define D31 D32 D32
#define D32 D33 D33
#define D33 D34 D34
#define D34 D35 D35
#define D35 D36 D36
#define D36 D37 D37
#define D37 D38 D38
#define D38 D39 D39
#define D39 D40 D40
#define D40 F(
#if D0 defined X)
int k;
#endif

#define C0 C1
#define C1 C2
#define C2 C3
#define C3 C4
#define C4 C5
#define C5 C6
#define C6 C7
#define C7 C8
#define C8 C9
#define C9 C10
#define C10 C11
#define C11 C12
#define C12 C13
#define C13 C14
#define C14 C15
#define C15 C16
#define C16 C17
#define C17 C18
#define C18 C19
#define C19 C20
#define C20 C21
#define C21 C22
#define C22 C23
#define C23 C24
#define C24 C25
#define C25 C26
#define C26 C27
#define C27 C28
#define C28 C29
#define C29 C30
#define C30 C31
#define C31 C32
#define C32 C33
#define C33 C34
#define C34 C35
#define C35 C36
#define C36 C37
#define C37 C38
#define C38 C39
#define C39 C40
#define C40 C41
#define C41 C42
#define C42 C43
#define C43 C44
#define C44 C45
#define C45 C46
#define C46 C47
#define C47 C48
#define C48 C49
#define C49 C50
#define C50 C51
#define C51 C52
#define C52 C53
#define C53 C54
#define C54 C55
#define C55 C56
#define C56 C57
#define C57 C58
#define C58 C59
#define C59 C60
#define C60 C61
#define C61 C62
#define C62 C63
#define C63 C64
#define C64 C65
#define C65 C66
#define C66 C67
#define C67 C68
#define C68 C69
#define C69 C70
#define C70 C71
#define C71 C72
#define C72 C73
#define C73 C74
#define C74 C75
#define C75 C76
#define C76 C77
#define C77 C78
#define C78 C79
#define C79 C80
#define C80 C81
#define C81 C82
#define C82 C83
#define C83 C84
#define C84 C85
#define C85 C86
#define C86 C87
#define C87 C88
#define C88 C89
#define C89 C90
#define C90 C91
#define C91 C92
#define C92 C93
#define C93 C94
#define C94 C95
#define C95 C96
#define C96 C97
#define C97 C98
#define C98 C99
#define C99 C100
#define C100 C101
#define C101 C102
#define C102 C103
#define C103 C104
#define C104 C105
#define C105 C106
#define C106 C107
#define C107 C108
#define C108 C109
#define C109 C110
#define C110 C111
#define C111 C112
#define C112 C113
#define C113 C114
#define C114 C115
#define C115 C116
#define C116 C117
#define C117 C118
#define C118 C119
#define C119 C120
#define C120 C121
#define C121 C122
#define C122 C123
#define C123 C124
#define C124 C125
#define C125 C126
#define C126 C127
#define C127 C128
#define C128 C129
#define C129 C130
#define C130 C131
#define C131 C132
#define C132 C133
#define C133 C134
#define C134 C135
#define C135 C136
#define C136 C137
#define C137 C138
#define C138 C139
#define C139 C140
#define C140 C141
#define C141 C142
#define C142 C143
#define C143 C144
#define C144 C145
#define C145 C146
#define C146 C147
#define C147 C148
#define C148 C149
#define C149 C150
#define C150 C151
#define C151 C152
#define C152 C153
#define C153 C154
#define C154 C155
#define C155 C156
#define C156 C157
#define C157 C158
#define C158 C159
#define C159 C160
#define C160 C161
#define C161 C162
#define C162 C163
#define C163 C164
#define C164 C165
#define C165 C166
#define C166 C167
#define C167 C168
#define C168 C169
#define C169 C170
#define C170 C171
#define C171 C172
#define C172 C173
#define C173 C174
#define C174 C175
#define C175 C176
#define C176 C177
#define C177 C178
#define C178 C179
#define C179 C180
#define C180 C181
#define C181 C182
#define C182 C183
#define C183 C184
#define C184 C185
#define C185 C186
#define C186 C187
#define C187 C188
#define C188 C189
#define C189 C190
#define C190 C191
#define C191 C192
#define C192 C193
#define C193 C194
#define C194 C195
#define C195 C196
#define C196 C197
#define C197 C198
#define C198 C199
#define C199 C200
#define C200 C201
#define C201 C202
#define C202 C203
#define C203 C204
#define C204 C205
#define C205 C206
#define C206 C207
#define C207 C208
#define C208 C209
#define C209 C210
#define C210 C211
#define C211 C212
#define C212 C213
#define C213 C214
#define C214 C215
#define C215 C216
#define C216 C217
#define C217 C218
#define C218 C219
#define C219 C220
#define C220 C221
#define C221 C222
#define C222 C223
#define C223 C224
#define C224 C225
#define C225 C226
#define C226 C227
#define C227 C228
#define C228 C229
#define C229 C230
#define C230 C231
#define C231 C232
#define C232 C233
#define C233 C234
#define C234 C235
#define C235 C236
#define C236 C237
#define C237 C238
#define C238 C239
#define C239 C240
#define C240 C241
#define C241 C242
#define C242 C243
#define C243 C244
#define C244 C245
#define C245 C246
#define C246 C247
#define C247 C248
#define C248 C249
#define C249 C250
#define C250 C251
#define C251 C252
#define C252 C253
#define C253 C254
#define C254 C255
#define C255 C256
#define C256 C257
#define C257 C258
#define C258 C259
#define C259 C260
#define C260 C261
#define C261 C262
#define C262 C263
#define C263 C264
#define C264 C265
#define C265 C266
#define C266 C267
#define C267 C268
#define C268 C269
#define C269 C270
#define C270 C271
#define C271 C272
#define C272 C273
#define C273 C274
#define C274 C275
#define C275 C276
#define C276 C277
#define C277 C278
#define C278 C279
#define C279 C280
#define C280 C281
#define C281 C282
#define C282 C283
#define C283 C284
#define C284 C285
#define C285 C286
#define C286 C287
#define C287 C288
#define C288 C289
#define C289 C290
#define C290 C291
#define C291 C292
#define C292 C293
#define C293 C294
#define C294 C295
#define C295 C296
#define C296 C297
#define C297 C298
#define C298 C299
#define C299 C300
#define C300 0
#if C0 + defined X
int l;
#endif
